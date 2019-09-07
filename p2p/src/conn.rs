// Copyright 2018 The Grin Developers
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

use crate::core::ser;
use crate::core::ser::{FixedLength, ProtocolVersion};
use crate::msg::{
	read_body, read_discard, read_header, read_item, write_to_buf, MsgHeader, MsgHeaderWrapper,
	Type,
};
use crate::types::Error;
use crate::util::{RateCounter, RwLock};
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use std::{
	cmp,
	thread::{self, JoinHandle},
};

const IO_TIMEOUT: Duration = Duration::from_millis(1000);

/// A trait to be implemented in order to receive messages from the
/// connection. Allows providing an optional response.
pub trait MessageHandler: Send + 'static {
	fn consume<'a>(
		&self,
		msg: Message<'a>,
		writer: &'a mut dyn Write,
		tracker: Arc<Tracker>,
	) -> Result<Option<Response<'a>>, Error>;
}

// Macro to simplify the boilerplate around async I/O error handling,
// especially with WouldBlock kind of errors.
macro_rules! try_break {
	($inner:expr) => {
		match $inner {
			Ok(v) => Some(v),
			Err(Error::Connection(ref e)) if e.kind() == io::ErrorKind::TimedOut => None,
			Err(Error::Connection(ref e)) if e.kind() == io::ErrorKind::WouldBlock => {
				// to avoid the heavy polling which will consume CPU 100%
				thread::sleep(Duration::from_millis(10));
				None
				}
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

/// Response to a `Message`.
pub struct Response<'a> {
	resp_type: Type,
	body: Vec<u8>,
	version: ProtocolVersion,
	stream: &'a mut dyn Write,
	attachment: Option<File>,
}

impl<'a> Response<'a> {
	pub fn new<T: ser::Writeable>(
		resp_type: Type,
		version: ProtocolVersion,
		body: T,
		stream: &'a mut dyn Write,
	) -> Result<Response<'a>, Error> {
		let body = ser::ser_vec(&body, version)?;
		Ok(Response {
			resp_type,
			body,
			version,
			stream,
			attachment: None,
		})
	}

	fn write(mut self, tracker: Arc<Tracker>) -> Result<(), Error> {
		let mut msg = ser::ser_vec(
			&MsgHeader::new(self.resp_type, self.body.len() as u64),
			self.version,
		)?;
		msg.append(&mut self.body);
		self.stream.write_all(&msg[..])?;
		tracker.inc_sent(msg.len() as u64);

		if let Some(mut file) = self.attachment {
			let mut buf = [0u8; 8000];
			loop {
				match file.read(&mut buf[..]) {
					Ok(0) => break,
					Ok(n) => {
						self.stream.write_all(&buf[..n])?;
						// Increase sent bytes "quietly" without incrementing the counter.
						// (In a loop here for the single attachment).
						tracker.inc_quiet_sent(n as u64);
					}
					Err(e) => return Err(From::from(e)),
				}
			}
		}
		Ok(())
	}

	pub fn add_attachment(&mut self, file: File) {
		self.attachment = Some(file);
	}
}

pub const SEND_CHANNEL_CAP: usize = 100;

pub struct StopHandle {
	/// Channel to close the connection
	stopped: Arc<AtomicBool>,
	// we need Option to take ownhership of the handle in stop()
	reader_thread: Option<JoinHandle<()>>,
	writer_thread: Option<JoinHandle<()>>,
}

impl StopHandle {
	/// Schedule this connection to safely close via the async close_channel.
	pub fn stop(&self) {
		self.stopped.store(true, Ordering::Relaxed);
	}

	pub fn wait(&mut self) {
		if let Some(reader_thread) = self.reader_thread.take() {
			self.join_thread(reader_thread);
		}
		if let Some(writer_thread) = self.writer_thread.take() {
			self.join_thread(writer_thread);
		}
	}

	fn join_thread(&self, peer_thread: JoinHandle<()>) {
		// wait only if other thread is calling us, eg shutdown
		if thread::current().id() != peer_thread.thread().id() {
			debug!("waiting for thread {:?} exit", peer_thread.thread().id());
			if let Err(e) = peer_thread.join() {
				error!("failed to stop peer thread: {:?}", e);
			}
		} else {
			debug!(
				"attempt to stop thread {:?} from itself",
				peer_thread.thread().id()
			);
		}
	}
}

pub struct ConnHandle {
	/// Channel to allow sending data through the connection
	pub send_channel: mpsc::SyncSender<Vec<u8>>,
}

impl ConnHandle {
	pub fn send<T>(&self, body: T, msg_type: Type, version: ProtocolVersion) -> Result<u64, Error>
	where
		T: ser::Writeable,
	{
		let buf = write_to_buf(body, msg_type, version)?;
		let buf_len = buf.len();
		self.send_channel.try_send(buf)?;
		Ok(buf_len as u64)
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

	pub fn inc_received(&self, size: u64) {
		self.received_bytes.write().inc(size);
	}

	pub fn inc_sent(&self, size: u64) {
		self.sent_bytes.write().inc(size);
	}

	pub fn inc_quiet_received(&self, size: u64) {
		self.received_bytes.write().inc_quiet(size);
	}

	pub fn inc_quiet_sent(&self, size: u64) {
		self.sent_bytes.write().inc_quiet(size);
	}
}

/// Start listening on the provided connection and wraps it. Does not hang
/// the current thread, instead just returns a future and the Connection
/// itself.
pub fn listen<H>(
	stream: TcpStream,
	version: ProtocolVersion,
	tracker: Arc<Tracker>,
	handler: H,
) -> io::Result<(ConnHandle, StopHandle)>
where
	H: MessageHandler,
{
	let (send_tx, send_rx) = mpsc::sync_channel(SEND_CHANNEL_CAP);

	stream
		.set_read_timeout(Some(IO_TIMEOUT))
		.expect("can't set read timeout");
	stream
		.set_write_timeout(Some(IO_TIMEOUT))
		.expect("can't set read timeout");

	let stopped = Arc::new(AtomicBool::new(false));

	let (reader_thread, writer_thread) =
		poll(stream, version, handler, send_rx, stopped.clone(), tracker)?;

	Ok((
		ConnHandle {
			send_channel: send_tx,
		},
		StopHandle {
			stopped,
			reader_thread: Some(reader_thread),
			writer_thread: Some(writer_thread),
		},
	))
}

fn poll<H>(
	conn: TcpStream,
	version: ProtocolVersion,
	handler: H,
	send_rx: mpsc::Receiver<Vec<u8>>,
	stopped: Arc<AtomicBool>,
	tracker: Arc<Tracker>,
) -> io::Result<(JoinHandle<()>, JoinHandle<()>)>
where
	H: MessageHandler,
{
	// Split out tcp stream out into separate reader/writer halves.
	let mut reader = conn.try_clone().expect("clone conn for reader failed");
	let mut writer = conn.try_clone().expect("clone conn for writer failed");
	let mut responder = conn.try_clone().expect("clone conn for writer failed");
	let reader_stopped = stopped.clone();

	let reader_thread = thread::Builder::new()
		.name("peer_read".to_string())
		.spawn(move || {
			loop {
				// check the read end
				match try_break!(read_header(&mut reader, version)) {
					Some(MsgHeaderWrapper::Known(header)) => {
						let msg = Message::from_header(header, &mut reader, version);

						trace!(
							"Received message header, type {:?}, len {}.",
							msg.header.msg_type,
							msg.header.msg_len
						);

						// Increase received bytes counter
						tracker.inc_received(MsgHeader::LEN as u64 + msg.header.msg_len);

						if let Some(Some(resp)) =
							try_break!(handler.consume(msg, &mut responder, tracker.clone()))
						{
							try_break!(resp.write(tracker.clone()));
						}
					}
					Some(MsgHeaderWrapper::Unknown(msg_len)) => {
						// Increase received bytes counter
						tracker.inc_received(MsgHeader::LEN as u64 + msg_len);

						try_break!(read_discard(msg_len, &mut reader));
					}
					None => {}
				}

				// check the close channel
				if reader_stopped.load(Ordering::Relaxed) {
					break;
				}
			}

			debug!(
				"Shutting down reader connection with {}",
				reader
					.peer_addr()
					.map(|a| a.to_string())
					.unwrap_or("?".to_owned())
			);
			let _ = reader.shutdown(Shutdown::Both);
		})?;

	let writer_thread = thread::Builder::new()
		.name("peer_write".to_string())
		.spawn(move || {
			let mut retry_send = Err(());
			loop {
				let maybe_data = retry_send.or_else(|_| send_rx.recv_timeout(IO_TIMEOUT));
				retry_send = Err(());
				if let Ok(data) = maybe_data {
					let written = try_break!(writer.write_all(&data[..]).map_err(&From::from));
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
		})?;
	Ok((reader_thread, writer_thread))
}
