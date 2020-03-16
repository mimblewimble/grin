// Copyright 2020 The Grin Developers
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

use crate::codec::{Codec, Output};
use crate::core::ser;
use crate::core::ser::ProtocolVersion;
use crate::msg::{write_message, Consume, Consumed, Msg, MsgHeader};
use crate::types::Error;
use crate::util::RateCounter;
use futures::channel::{mpsc, oneshot};
use futures::executor::block_on;
use futures::StreamExt;
use std::io;
use std::net::Shutdown;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::codec::FramedRead;

pub const SEND_CHANNEL_CAP: usize = 100;

type StopTx = oneshot::Sender<()>;

/// A trait to be implemented in order to receive messages from the
/// connection. Allows providing an optional response.
pub trait MessageHandler: Send + 'static {
	fn consume(&self, input: Consume, tracker: Arc<Tracker>) -> Result<Consumed, Error>;
}

// Macro to simplify the boilerplate around I/O and Grin error handling
macro_rules! try_break {
	($inner:expr) => {
		match $inner {
			Ok(v) => Some(v),
			Err(Error::Store(_))
			| Err(Error::Chain(_))
			| Err(Error::Internal)
			| Err(Error::NoDandelionRelay) => {
				debug!("error to none");
				None
				}
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

pub struct StopHandle {
	/// Channel to close the connection
	stop: Option<(StopTx, JoinHandle<()>)>,
}

impl StopHandle {
	/// Schedule this connection to safely close via the async close_channel.
	pub fn stop(&mut self) -> Result<(), ()> {
		if let Some((t, h)) = self.stop.take() {
			let _ = t.send(());
			let _ = block_on(h);
		}
		Ok(())
	}

	pub async fn wait(&mut self) {
		if let Some((t, h)) = self.stop.take() {
			let _ = t.send(());
			let _ = h.await;
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
	conn: TcpStream,
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

	let (stop_tx, join) = poll(
		conn,
		conn_handle.clone(),
		version,
		handler,
		send_rx,
		tracker,
	);

	Ok((
		conn_handle,
		StopHandle {
			stop: Some((stop_tx, join)),
		},
	))
}

fn poll<H>(
	mut conn: TcpStream,
	conn_handle: ConnHandle,
	version: ProtocolVersion,
	handler: H,
	send_rx: mpsc::Receiver<Msg>,
	tracker: Arc<Tracker>,
) -> (StopTx, JoinHandle<()>)
where
	H: MessageHandler,
{
	let peer_address = conn
		.peer_addr()
		.map(|a| a.to_string())
		.unwrap_or("?".to_owned());

	// Split out tcp stream out into separate reader/writer halves.
	let (stop_tx, stop_rx) = oneshot::channel();

	let join = tokio::spawn(async move {
		let (reader, writer) = conn.split();
		let reader = read(reader, conn_handle, version, handler, tracker.clone());
		let writer = write(writer, send_rx, tracker);

		tokio::select! {
			res = reader => {
				if let Err(e) = res {
					debug!("Reader connection with {} closed: {}", peer_address, e);
				}
				else {
					debug!("Reader connection with {} closed", peer_address);
				}
			}
			_ = writer => debug!("Writer connection with {} closed", peer_address),
			_ = stop_rx => {}
		};

		let _ = conn.shutdown(Shutdown::Both);

		debug!("Shutting down connection with {}", peer_address);
	});

	(stop_tx, join)
}

async fn read<H>(
	reader: ReadHalf<'_>,
	mut conn_handle: ConnHandle,
	version: ProtocolVersion,
	handler: H,
	tracker: Arc<Tracker>,
) -> io::Result<()>
where
	H: MessageHandler,
{
	let mut framed = FramedRead::new(reader, Codec::new(version));
	let mut attachment: Option<File> = None;
	loop {
		let tracker = tracker.clone();
		let mut next = try_next!(framed.next().await);
		let consume = match &mut next {
			Some(Output::Known(header, body)) => {
				trace!(
					"Received message header, type {:?}, len {}.",
					header.msg_type,
					header.msg_len
				);

				// Increase received bytes counter
				tracker
					.inc_received(MsgHeader::LEN as u64 + header.msg_len)
					.await;

				Consume::Message(header, ser::BufReader::new(body, version))
			}
			Some(Output::Unknown(len, type_byte)) => {
				debug!(
					"Received unknown message header, type {:?}, len {}.",
					type_byte, len
				);

				// Increase received bytes counter
				tracker.inc_received(MsgHeader::LEN as u64 + *len).await;

				continue;
			}
			Some(Output::Attachment(update, bytes)) => {
				let a = match &mut attachment {
					Some(a) => a,
					None => break,
				};

				a.write_all(&bytes).await?;
				if update.left == 0 {
					a.sync_all().await?;
					attachment = None;
				}

				Consume::Attachment(update)
			}
			None => continue,
		};
		debug!("Consume: {:?}", consume);

		// TODO: non-blocking handler
		let block = tokio::task::block_in_place(|| handler.consume(consume, tracker));
		if let Some(consumed) = try_break!(block) {
			debug!("Consumed: {:?}", consumed);
			match consumed {
				Consumed::Response(resp_msg) => {
					try_break!(conn_handle.send(resp_msg));
				}
				Consumed::Attachment(meta, file) => {
					// Start attachment
					framed.decoder_mut().expect_attachment(meta);
					attachment = Some(file);
				}
				Consumed::Disconnect => break,
				Consumed::None => {}
			}
		}
	}

	Ok(())
}

async fn write(mut writer: WriteHalf<'_>, mut rx: mpsc::Receiver<Msg>, tracker: Arc<Tracker>) {
	while let Some(msg) = rx.next().await {
		try_break!(write_message(&mut writer, &msg, tracker.clone()).await);
	}
}
