// Copyright 2019 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Provides a connection wrapper that handles the lower level tasks in sending
//! or receiving data from the TCP socket, as well as dealing with timeouts.
//!
//! Because of a few idiosyncracies in the Rust `TcpStream`, this has to use
//! async I/O to be able to both read *and* write on the connection. Which
//! forces us to go through some additional gymnastic to loop over the async
//! stream and make sure we get the right number of bytes out.

use crate::codec::Codec;
use crate::core::ser;
use crate::core::ser::{FixedLength, ProtocolVersion};
use crate::msg::{
	read_body, read_item, write_message, Msg, MsgHeader,
	MsgWrapper::{self, *},
};
use crate::types::Error;
use crate::util::RateCounter;
use bytes::Bytes;
use futures::channel::{mpsc, oneshot};
use futures::stream::{select, SplitSink, SplitStream};
use futures::{FutureExt, SinkExt, StreamExt};
use std::cmp;
use std::io::{self, Cursor, Read, Write};
use std::net::Shutdown;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::codec::Framed;

pub const SEND_CHANNEL_CAP: usize = 100;

type StopTx = oneshot::Sender<()>;
type StopRx = oneshot::Receiver<()>;
type Reader = SplitStream<Framed<TcpStream, Codec>>;
type Writer = SplitSink<Framed<TcpStream, Codec>, Bytes>;

/// A trait to be implemented in order to receive messages from the
/// connection. Allows providing an optional response.
pub trait MessageHandler: Send + 'static {
	fn consume<'a>(
		&self,
		msg: Message<'a>,
		stopped: Arc<AtomicBool>,
		tracker: Arc<Tracker>,
	) -> Result<Option<Msg>, Error>;
}

// Macro to simplify the boilerplate around I/O and Grin error handling
macro_rules! try_break {
	($inner:expr) => {
		match $inner {
			Ok(v) => Some(v),
			Err(Error::Store(_))
			| Err(Error::Chain(_))
			| Err(Error::Internal)
			| Err(Error::NoDandelionRelay) => None,
			Err(ref e) => {
				debug!("try_break: exit the loop: {:?}", e);
				break;
				}
			}
	};
}

macro_rules! try_next {
	($inner:expr) => {
		match $inner {
			Some(v) => try_break!(v),
			None => break,
			}
	};
}

/// A message as received by the connection. Provides access to the message
/// header lazily consumes the message body, handling its deserialization.
pub struct Message<'a> {
	pub header: MsgHeader,
	stream: &'a mut dyn Read,
	version: ProtocolVersion,
}

impl<'a> Message<'a> {
	fn from_header(
		header: MsgHeader,
		stream: &'a mut dyn Read,
		version: ProtocolVersion,
	) -> Message<'a> {
		Message {
			header,
			stream,
			version,
		}
	}

	/// Read the message body from the underlying connection
	pub fn body<T: ser::Readable>(&mut self) -> Result<T, Error> {
		read_body(&self.header, self.stream, self.version)
	}

	/// Read a single "thing" from the underlying connection.
	/// Return the thing and the total bytes read.
	pub fn streaming_read<T: ser::Readable>(&mut self) -> Result<(T, u64), Error> {
		read_item(self.stream, self.version)
	}

	pub fn copy_attachment(&mut self, len: usize, writer: &mut dyn Write) -> Result<usize, Error> {
		let mut written = 0;
		while written < len {
			let read_len = cmp::min(8000, len - written);
			let mut buf = vec![0u8; read_len];
			self.stream.read_exact(&mut buf[..])?;
			writer.write_all(&mut buf)?;
			written += read_len;
		}
		Ok(written)
	}
}

pub struct StopHandle {
	/// Channel to close the connection
	stop: Option<(StopTx, StopTx)>,
	join: Option<(JoinHandle<Reader>, JoinHandle<Writer>)>,
}

impl StopHandle {
	/// Schedule this connection to safely close via the async close_channel.
	pub fn stop(&mut self) -> Result<(), ()> {
		if let Some((r, w)) = self.stop.take() {
			let _ = r.send(());
			let _ = w.send(());
		}
		Ok(())
	}

	pub async fn wait(&mut self) {
		if let Some((r, w)) = self.join.take() {
			match (r.await, w.await) {
				(Ok(r), Ok(w)) => {
					// TODO: do we need this for graceful shutdown?
					let stream = r.reunite(w).expect("Unable to reunite stream").into_inner();
					let _ = stream.shutdown(Shutdown::Both);
				}
				_ => {}
			}
		}
	}
}

#[derive(Clone)]
pub struct ConnHandle {
	/// Channel to allow sending data through the connection
	pub send_channel: mpsc::Sender<Msg>,
}

impl ConnHandle {
	/// Send msg via the synchronous, bounded channel (sync_sender).
	/// Two possible failure cases -
	/// * Disconnected: Propagate this up to the caller so the peer connection can be closed.
	/// * Full: Our internal msg buffer is full. This is not a problem with the peer connection
	/// and we do not want to close the connection. We drop the msg rather than blocking here.
	/// If the buffer is full because there is an underlying issue with the peer
	/// and potentially the peer connection. We assume this will be handled at the peer level.
	pub fn send(&mut self, msg: Msg) -> Result<(), Error> {
		match self.send_channel.try_send(msg) {
			Ok(()) => Ok(()),
			Err(e) => {
				if e.is_disconnected() {
					Err(Error::Send("try_send disconnected".to_owned()))
				} else {
					debug!("conn_handle: try_send but buffer is full, dropping msg");
					Ok(())
				}
			}
		}
	}
}

pub struct Tracker {
	/// Bytes we've sent.
	pub sent_bytes: Arc<RwLock<RateCounter>>,
	/// Bytes we've received.
	pub received_bytes: Arc<RwLock<RateCounter>>,
}

impl Tracker {
	pub fn new() -> Tracker {
		let received_bytes = Arc::new(RwLock::new(RateCounter::new()));
		let sent_bytes = Arc::new(RwLock::new(RateCounter::new()));
		Tracker {
			received_bytes,
			sent_bytes,
		}
	}

	pub async fn inc_received(&self, size: u64) {
		self.received_bytes.write().await.inc(size);
	}

	pub async fn inc_sent(&self, size: u64) {
		self.sent_bytes.write().await.inc(size);
	}

	pub async fn inc_quiet_received(&self, size: u64) {
		self.received_bytes.write().await.inc_quiet(size);
	}

	pub async fn inc_quiet_sent(&self, size: u64) {
		self.sent_bytes.write().await.inc_quiet(size);
	}
}

/// Start listening on the provided connection and wraps it. Does not hang
/// the current thread, instead just returns a future and the Connection
/// itself.
pub async fn listen<H>(
	framed: Framed<TcpStream, Codec>,
	version: ProtocolVersion,
	tracker: Arc<Tracker>,
	handler: H,
) -> io::Result<(ConnHandle, StopHandle)>
where
	H: MessageHandler,
{
	let (send_tx, send_rx) = mpsc::channel(SEND_CHANNEL_CAP);

	let conn_handle = ConnHandle {
		send_channel: send_tx,
	};

	let (stop_read, stop_write, join_read, join_write) = poll(
		framed,
		conn_handle.clone(),
		version,
		handler,
		send_rx,
		tracker,
	)?;

	Ok((
		conn_handle,
		StopHandle {
			stop: Some((stop_read, stop_write)),
			join: Some((join_read, join_write)),
		},
	))
}

fn poll<H>(
	framed: Framed<TcpStream, Codec>,
	conn_handle: ConnHandle,
	version: ProtocolVersion,
	handler: H,
	send_rx: mpsc::Receiver<Msg>,
	tracker: Arc<Tracker>,
) -> io::Result<(StopTx, StopTx, JoinHandle<Reader>, JoinHandle<Writer>)>
where
	H: MessageHandler,
{
	let peer_address = framed
		.get_ref()
		.peer_addr()
		.map(|a| a.to_string())
		.unwrap_or("?".to_owned());

	// Split out tcp stream out into separate reader/writer halves.
	let (writer, reader) = framed.split();

	let (stop_read_tx, stop_read_rx) = oneshot::channel();
	let (stop_write_tx, stop_write_rx) = oneshot::channel();

	let read_handle = tokio::spawn(read(
		reader,
		conn_handle,
		stop_read_rx,
		version,
		handler,
		tracker.clone(),
		peer_address.clone(),
	));
	let write_handle = tokio::spawn(write(
		writer,
		send_rx,
		stop_write_rx,
		version,
		tracker,
		peer_address,
	));

	Ok((stop_read_tx, stop_write_tx, read_handle, write_handle))
}

async fn read<H>(
	reader: Reader,
	mut conn_handle: ConnHandle,
	stop: StopRx,
	version: ProtocolVersion,
	handler: H,
	tracker: Arc<Tracker>,
	peer_address: String,
) -> Reader
where
	H: MessageHandler,
{
	enum Reading {
		Message(MsgWrapper),
		Stop,
	}
	use self::Message as Wrapper;
	use Reading::*;

	let reader = reader.map(|msg| msg.map(|m| Message(m)));
	let stop = stop.into_stream().map(|_| Ok(Stop));
	let mut select = select(reader, stop);
	let atomic = Arc::new(AtomicBool::new(false));
	loop {
		let atomic = atomic.clone();
		let tracker = tracker.clone();
		match try_next!(select.next().await) {
			Some(Message(Known(msg))) => {
				trace!(
					"Received message header, type {:?}, len {}.",
					msg.header.msg_type,
					msg.header.msg_len
				);

				// Increase received bytes counter
				tracker
					.inc_received(MsgHeader::LEN as u64 + msg.header.msg_len)
					.await;

				let (header, body, version) = msg.parts();
				let mut cursor = Cursor::new(body);
				let wrap = Wrapper::from_header(header, &mut cursor, version);
				// TODO: non-blocking handler
				let block = tokio::task::block_in_place(|| handler.consume(wrap, atomic, tracker));
				if let Some(Some(resp_msg)) = try_break!(block) {
					try_break!(conn_handle.send(resp_msg));
				}
			}
			Some(Message(Unknown(len, type_byte))) => {
				debug!(
					"Received unknown message header, type {:?}, len {}.",
					type_byte, len
				);
				// Increase received bytes counter
				tracker.inc_received(MsgHeader::LEN as u64 + len).await;
			}
			Some(Stop) => {
				// Receive stop signal
				break;
			}
			None => {}
		}
	}

	debug!("Shutting down reader connection with {}", peer_address);
	let (reader, _) = select.into_inner();
	reader.into_inner()
}

async fn write(
	mut writer: Writer,
	rx: mpsc::Receiver<Msg>,
	stop: StopRx,
	version: ProtocolVersion,
	tracker: Arc<Tracker>,
	peer_address: String,
) -> Writer {
	enum Writing {
		Message(Msg),
		Stop,
	}
	use Writing::*;

	let rx = rx.map(|m| Message(m));
	let stop = stop.into_stream().map(|_| Stop);
	let mut select = select(rx, stop);
	while let Some(item) = select.next().await {
		match item {
			Message(msg) => {
				try_break!(write_message(&mut writer, &msg, tracker.clone()).await);
			}
			Stop => break,
		}
	}

	writer

	/*let writer_thread = thread::Builder::new()
	.name("peer_write".to_string())
	.spawn(move || {
		let mut retry_send = Err(());
		writer
			.set_write_timeout(Some(BODY_IO_TIMEOUT))
			.expect("set timeout");
		loop {
			let maybe_data = retry_send.or_else(|_| send_rx.recv_timeout(CHANNEL_TIMEOUT));
			retry_send = Err(());
			if let Ok(data) = maybe_data {
				let written =
					try_break!(write_message(&mut writer, &data, writer_tracker.clone()));
				if written.is_none() {
					retry_send = Ok(data);
				}
			}
			// check the close channel
			if stopped.load(Ordering::Relaxed) {
				break;
			}
		}

		debug!(
			"Shutting down writer connection with {}",
			writer
				.peer_addr()
				.map(|a| a.to_string())
				.unwrap_or("?".to_owned())
		);
	})?;*/
}
