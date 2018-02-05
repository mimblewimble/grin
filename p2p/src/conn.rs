// Copyright 2016-2018 The Grin Developers
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

use std::io::{self, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::net::TcpStream;
use std::thread;
use std::time;

use core::ser;
use msg::*;
use types::*;
use util::LOGGER;

pub trait MessageHandler: Send + 'static {
	fn consume(&self, msg: &mut Message) -> Result<Option<(Vec<u8>, Type)>, Error>;
}

// Macro to simplify the boilerplate around asyn I/O error handling,
// especially with WouldBlock kind of errors.
macro_rules! try_break {
	($chan:ident, $inner:expr) => {
		match $inner {
			Ok(v) => Some(v),
			Err(Error::Connection(ref e)) if e.kind() == io::ErrorKind::WouldBlock => {
				None
			}
			Err(e) => {
				let _ = $chan.send(e);
				break;
			}
		}
	}
}

pub struct Message<'a> {
	pub header: MsgHeader,
	conn: &'a mut TcpStream,
}

impl<'a> Message<'a> {

	fn from_header(header: MsgHeader, conn: &'a mut TcpStream) -> Message<'a> {
		Message{header, conn}
	}

	pub fn body<T>(&mut self) -> Result<T, Error> where T: ser::Readable {
		read_body(&self.header, self.conn)
	}
}

// TODO count sent and received
pub struct Tracker {
	/// Bytes we've sent.
	pub sent_bytes: Arc<Mutex<u64>>,
	/// Bytes we've received.
	pub received_bytes: Arc<Mutex<u64>>,
	/// Channel to allow sending data through the connection
	pub send_channel: mpsc::Sender<Vec<u8>>,
	/// Channel to close the connection
	pub close_channel: mpsc::Sender<()>,
	/// Channel to check for errors on the connection
	pub error_channel: mpsc::Receiver<Error>,
}

impl Tracker {
	pub fn send<T>(&self, body: T, msg_type: Type) -> Result<(), Error>
	where
		T: ser::Writeable
	{
		let (header_buf, body_buf) = write_to_bufs(body, msg_type);
		self.send_channel.send(header_buf)?;
		self.send_channel.send(body_buf)?;
		Ok(())
	}
}

/// Start listening on the provided connection and wraps it. Does not hang
/// the current thread, instead just returns a future and the Connection
/// itself.
pub fn listen<H>(stream: TcpStream, handler: H) -> Tracker
where
	H: MessageHandler,
{
	let (send_tx, send_rx) = mpsc::channel();
	let (close_tx, close_rx) = mpsc::channel();
	let (error_tx, error_rx) = mpsc::channel();

	stream.set_nonblocking(true).expect("Non-blocking IO not available.");
	poll(stream, handler, send_rx, send_tx.clone(), error_tx, close_rx);

	Tracker {
		sent_bytes: Arc::new(Mutex::new(0)),
		received_bytes: Arc::new(Mutex::new(0)),
		send_channel: send_tx,
		close_channel: close_tx,
		error_channel: error_rx,
	}
}

fn poll<H>(
	conn: TcpStream,
	handler: H,
	send_rx: mpsc::Receiver<Vec<u8>>,
	send_tx: mpsc::Sender<Vec<u8>>,
	error_tx: mpsc::Sender<Error>,
	close_rx: mpsc::Receiver<()>
)
where
	H: MessageHandler,
{

	let mut conn = conn;
	let _ = thread::Builder::new().name("peer".to_string()).spawn(move || {
		let sleep_time = time::Duration::from_millis(1);

		let conn = &mut conn;
		let mut retry_send = Err(());
		loop {
			// check the read end
			if let Some(h) = try_break!(error_tx, read_header(conn)) {
				let mut msg = Message::from_header(h, conn);
				debug!(LOGGER, "Received message header, type {:?}, len {}.", msg.header.msg_type, msg.header.msg_len);
				if let Some(Some((body, typ))) = try_break!(error_tx, handler.consume(&mut msg)) {
					respond(&send_tx, typ, body);
				}
			}

			// check the write end
			if let Ok::<Vec<u8>, ()>(data) = retry_send {
				if let None = try_break!(error_tx, conn.write_all(&data[..]).map_err(&From::from)) {
					retry_send = Ok(data);
				} else {
					retry_send = Err(());
				}
			} else if let Ok(data) = send_rx.try_recv() {
				if let None = try_break!(error_tx, conn.write_all(&data[..]).map_err(&From::from)) {
					retry_send = Ok(data);
				} else {
					retry_send = Err(());
				}
			} else {
				retry_send = Err(());
			}

			// check the close channel
			if let Ok(_) = close_rx.try_recv() {
				debug!(LOGGER,
							 "Connection close with {} initiated by us",
							 conn.peer_addr().map(|a| a.to_string()).unwrap_or("?".to_owned()));
				break;
			}

			thread::sleep(sleep_time);
		}
	});
}

fn respond(send_tx: &mpsc::Sender<Vec<u8>>, msg_type: Type, body: Vec<u8>) {
	let header = ser::ser_vec(&MsgHeader::new(msg_type, body.len() as u64)).unwrap();
	send_tx.send(header).unwrap();
	send_tx.send(body).unwrap();
}
