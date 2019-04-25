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
use crate::core::ser::FixedLength;
use crate::msg::{read_body, read_header, read_item, write_to_buf, MsgHeader, Type};
use crate::types::Error;
use crate::util::{RateCounter, RwLock};
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use std::{cmp, thread};

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
	($chan:ident, $inner:expr) => {
		match $inner {
			Ok(v) => Some(v),
			Err(Error::Connection(ref e))
				if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
				{
				None
				}
			Err(e) => {
				let _ = $chan.send(e);
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
}

impl<'a> Message<'a> {
	fn from_header(header: MsgHeader, stream: &'a mut dyn Read) -> Message<'a> {
		Message { header, stream }
	}

	/// Read the message body from the underlying connection
	pub fn body<T: ser::Readable>(&mut self) -> Result<T, Error> {
		read_body(&self.header, self.stream)
	}

	/// Read a single "thing" from the underlying connection.
	/// Return the thing and the total bytes read.
	pub fn streaming_read<T: ser::Readable>(&mut self) -> Result<(T, u64), Error> {
		read_item(self.stream)
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
	stream: &'a mut dyn Write,
	attachment: Option<File>,
}

impl<'a> Response<'a> {
	pub fn new<T: ser::Writeable>(
		resp_type: Type,
		body: T,
		stream: &'a mut dyn Write,
	) -> Result<Response<'a>, Error> {
		let body = ser::ser_vec(&body)?;
		Ok(Response {
			resp_type,
			body,
			stream,
			attachment: None,
		})
	}

	fn write(mut self, tracker: Arc<Tracker>) -> Result<(), Error> {
		let mut msg = ser::ser_vec(&MsgHeader::new(self.resp_type, self.body.len() as u64))?;
		msg.append(&mut self.body);
		self.stream.write_all(&msg[..])?;
		tracker.sent_bytes_inc(msg.len() as u64);
		if let Some(mut file) = self.attachment {
			let mut buf = [0u8; 8000];
			loop {
				match file.read(&mut buf[..]) {
					Ok(0) => break,
					Ok(n) => {
						self.stream.write_all(&buf[..n])?;
						// Increase sent bytes "quietly" without incrementing the counter.
						// (In a loop here for the single attachment).
						tracker.sent_bytes_inc_quiet(n as u64);
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

pub const SEND_CHANNEL_CAP: usize = 10;

pub struct Tracker {
	/// Bytes we've sent.
	pub sent_bytes: Arc<RwLock<RateCounter>>,
	/// Bytes we've received.
	pub received_bytes: Arc<RwLock<RateCounter>>,
	/// Channel to allow sending data through the connection
	pub send_channel: mpsc::SyncSender<Vec<u8>>,
	/// Channel to check for errors on the connection
	//pub error_channel: mpsc::Receiver<Error>,
	closed: AtomicBool,
	pub number_threads: AtomicU8,
}

impl Tracker {
	pub fn send<T>(&self, body: T, msg_type: Type) -> Result<(), Error>
	where
		T: ser::Writeable,
	{
		let buf = write_to_buf(body, msg_type)?;
		let buf_len = buf.len();
		self.send_channel.try_send(buf)?;
		self.sent_bytes_inc(buf_len as u64);
		Ok(())
	}

	pub fn is_closed(&self) -> bool {
		self.closed.load(Ordering::Relaxed)
	}

	pub fn close(&self) {
		self.closed.store(true, Ordering::Relaxed)
	}

	pub fn received_bytes_inc(&self, bytes: u64) {
		let mut received_bytes = self.received_bytes.write();
		received_bytes.inc(bytes);
	}

	pub fn received_bytes_inc_quiet(&self, bytes: u64) {
		let mut received_bytes = self.received_bytes.write();
		received_bytes.inc_quiet(bytes);
	}

	pub fn sent_bytes_inc(&self, bytes: u64) {
		let mut sent_bytes = self.sent_bytes.write();
		sent_bytes.inc(bytes);
	}

	pub fn sent_bytes_inc_quiet(&self, bytes: u64) {
		let mut sent_bytes = self.sent_bytes.write();
		sent_bytes.inc_quiet(bytes);
	}
}
const IO_TIMEOUT: Duration = Duration::from_millis(1000);

/// Start listening on the provided connection and wraps it. Does not hang
/// the current thread, instead just returns a future and the Connection
/// itself.
pub fn listen<H>(stream: TcpStream, handler: H) -> (Arc<Tracker>, mpsc::Receiver<Error>)
where
	H: MessageHandler,
{
	let (send_tx, send_rx) = mpsc::sync_channel(SEND_CHANNEL_CAP);
	let (error_tx, error_rx) = mpsc::channel();

	// Counter of number of bytes received
	let received_bytes = Arc::new(RwLock::new(RateCounter::new()));
	// Counter of number of bytes sent
	let sent_bytes = Arc::new(RwLock::new(RateCounter::new()));

	stream
		.set_read_timeout(Some(IO_TIMEOUT))
		.expect("can't set read timeout");
	stream
		.set_write_timeout(Some(IO_TIMEOUT))
		.expect("can't set write timeout");
	//stream
	//	.set_nonblocking(true)
	//	.expect("Non-blocking IO not available.");
	//

	let tracker = Arc::new(Tracker {
		sent_bytes: sent_bytes.clone(),
		received_bytes: received_bytes.clone(),
		send_channel: send_tx,
		closed: AtomicBool::new(false),
		number_threads: AtomicU8::new(0),
	});
	poll(stream, handler, send_rx, error_tx, tracker.clone());
	(tracker, error_rx)
}

fn poll<H>(
	conn: TcpStream,
	handler: H,
	send_rx: mpsc::Receiver<Vec<u8>>,
	error_tx: mpsc::Sender<Error>,
	tracker: Arc<Tracker>,
) where
	H: MessageHandler,
{
	// Split out tcp stream out into separate reader/writer halves.
	let mut reader = conn.try_clone().expect("clone conn for reader failed");
	let mut responder = conn.try_clone().expect("clone conn for reader failed");
	let mut writer = conn.try_clone().expect("clone conn for writer failed");

	let tracker_read = tracker.clone();
	let error_read_tx = error_tx.clone();
	let _ = thread::Builder::new()
		.name("peer_read".to_string())
		.spawn(move || {
			tracker_read.number_threads.fetch_add(1, Ordering::Relaxed);
			loop {
				if let Some(h) = try_break!(error_read_tx, read_header(&mut reader, None)) {
					let msg = Message::from_header(h, &mut reader);

					trace!(
						"Received message header, type {:?}, len {}.",
						msg.header.msg_type,
						msg.header.msg_len
					);

					// Increase received bytes counter
					tracker_read.received_bytes_inc(MsgHeader::LEN as u64 + msg.header.msg_len);

					if let Some(Some(resp)) = try_break!(
						error_read_tx,
						handler.consume(msg, &mut responder, tracker_read.clone())
					) {
						try_break!(error_read_tx, resp.write(tracker_read.clone()));
					}
				}

				if tracker_read.is_closed() {
					debug!(
						"Connection close with {} initiated by us",
						conn.peer_addr()
							.map(|a| a.to_string())
							.unwrap_or("?".to_owned())
					);
					break;
				}
			}
			tracker_read.number_threads.fetch_sub(1, Ordering::Relaxed);
			let _ = conn.shutdown(Shutdown::Both);
		});

	let _ = thread::Builder::new()
		.name("peer_write".to_string())
		.spawn(move || {
			tracker.number_threads.fetch_add(1, Ordering::Relaxed);
			let mut retry_send = Err(());
			// check the write end, use or_else so try_recv is lazily eval'd
			loop {
				let maybe_data = retry_send.or_else(|_| send_rx.recv_timeout(IO_TIMEOUT));
				retry_send = Err(());
				if let Ok(data) = maybe_data {
					let written =
						try_break!(error_tx, writer.write_all(&data[..]).map_err(&From::from));
					if written.is_none() {
						retry_send = Ok(data);
					}
				}
				if tracker.is_closed() {
					debug!("Connection close with initiated by us, closing writer end",);
					break;
				}
			}
			tracker.number_threads.fetch_sub(1, Ordering::Relaxed);
		});
}
