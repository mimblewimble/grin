// Copyright 2021 The Grin Developers
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

use crate::codec::{Codec, BODY_IO_TIMEOUT};
use crate::core::ser::ProtocolVersion;
use crate::msg::{write_message, Consumed, Message, Msg};
use crate::types::Error;
use crate::util::{RateCounter, RwLock};
use std::fs::File;
use std::io::{self, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub const SEND_CHANNEL_CAP: usize = 100;

const CHANNEL_TIMEOUT: Duration = Duration::from_millis(1000);

/// A trait to be implemented in order to receive messages from the
/// connection. Allows providing an optional response.
pub trait MessageHandler: Send + 'static {
	fn consume(&self, message: Message) -> Result<Consumed, Error>;
}

// Macro to simplify the boilerplate around I/O and Grin error handling
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

#[derive(Clone)]
pub struct ConnHandle {
	/// Channel to allow sending data through the connection
	pub send_channel: mpsc::SyncSender<Msg>,
}

impl ConnHandle {
	/// Send msg via the synchronous, bounded channel (sync_sender).
	/// Two possible failure cases -
	/// * Disconnected: Propagate this up to the caller so the peer connection can be closed.
	/// * Full: Our internal msg buffer is full. This is not a problem with the peer connection
	/// and we do not want to close the connection. We drop the msg rather than blocking here.
	/// If the buffer is full because there is an underlying issue with the peer
	/// and potentially the peer connection. We assume this will be handled at the peer level.
	pub fn send(&self, msg: Msg) -> Result<(), Error> {
		match self.send_channel.try_send(msg) {
			Ok(()) => Ok(()),
			Err(mpsc::TrySendError::Disconnected(_)) => {
				Err(Error::Send("try_send disconnected".to_owned()))
			}
			Err(mpsc::TrySendError::Full(_)) => {
				debug!("conn_handle: try_send but buffer is full, dropping msg");
				Ok(())
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

	let stopped = Arc::new(AtomicBool::new(false));

	let conn_handle = ConnHandle {
		send_channel: send_tx,
	};

	let (reader_thread, writer_thread) = poll(
		stream,
		conn_handle.clone(),
		version,
		handler,
		send_rx,
		stopped.clone(),
		tracker,
	)?;

	Ok((
		conn_handle,
		StopHandle {
			stopped,
			reader_thread: Some(reader_thread),
			writer_thread: Some(writer_thread),
		},
	))
}

fn poll<H>(
	conn: TcpStream,
	conn_handle: ConnHandle,
	version: ProtocolVersion,
	handler: H,
	send_rx: mpsc::Receiver<Msg>,
	stopped: Arc<AtomicBool>,
	tracker: Arc<Tracker>,
) -> io::Result<(JoinHandle<()>, JoinHandle<()>)>
where
	H: MessageHandler,
{
	// Split out tcp stream out into separate reader/writer halves.
	let reader = conn.try_clone().expect("clone conn for reader failed");
	let mut writer = conn.try_clone().expect("clone conn for writer failed");
	let reader_stopped = stopped.clone();

	let reader_tracker = tracker.clone();
	let writer_tracker = tracker;

	let reader_thread = thread::Builder::new()
		.name("peer_read".to_string())
		.spawn(move || {
			let peer_addr = reader
				.peer_addr()
				.map(|a| a.to_string())
				.unwrap_or_else(|_| "?".to_owned());
			let mut codec = Codec::new(version, reader);
			let mut attachment: Option<File> = None;
			loop {
				// check the close channel
				if reader_stopped.load(Ordering::Relaxed) {
					break;
				}

				// check the read end
				let (next, bytes_read) = codec.read();

				// increase the appropriate counter
				match &next {
					Ok(Message::Attachment(_, _)) => reader_tracker.inc_quiet_received(bytes_read),
					Ok(Message::Headers(data)) => {
						// We process a full 512 headers locally in smaller 32 header batches.
						// We only want to increment the msg count once for the full 512 headers.
						if data.remaining == 0 {
							reader_tracker.inc_received(bytes_read);
						} else {
							reader_tracker.inc_quiet_received(bytes_read);
						}
					}
					_ => reader_tracker.inc_received(bytes_read),
				}

				let message = match try_break!(next) {
					Some(Message::Unknown(type_byte)) => {
						debug!(
							"Received unknown message, type {:?}, len {}.",
							type_byte, bytes_read
						);
						continue;
					}
					Some(Message::Attachment(update, bytes)) => {
						let a = match &mut attachment {
							Some(a) => a,
							None => {
								error!("Received unexpected attachment chunk");
								break;
							}
						};

						let bytes = bytes.unwrap();
						if let Err(e) = a.write_all(&bytes) {
							error!("Unable to write attachment file: {}", e);
							break;
						}
						if update.left == 0 {
							if let Err(e) = a.sync_all() {
								error!("Unable to sync attachment file: {}", e);
								break;
							}
							attachment.take();
						}

						Message::Attachment(update, None)
					}
					Some(message) => {
						trace!("Received message, type {}, len {}.", message, bytes_read);
						message
					}
					None => continue,
				};

				let consumed = try_break!(handler.consume(message)).unwrap_or(Consumed::None);
				match consumed {
					Consumed::Response(resp_msg) => {
						try_break!(conn_handle.send(resp_msg));
					}
					Consumed::Attachment(meta, file) => {
						// Start attachment
						codec.expect_attachment(meta);
						attachment = Some(file);
					}
					Consumed::Disconnect => break,
					Consumed::None => {}
				}
			}

			debug!("Shutting down reader connection with {}", peer_addr);
			let _ = codec.stream().shutdown(Shutdown::Both);
		})?;

	let writer_thread = thread::Builder::new()
		.name("peer_write".to_string())
		.spawn(move || {
			let mut retry_send = Err(());
			let _ = writer.set_write_timeout(Some(BODY_IO_TIMEOUT));
			loop {
				let maybe_data = retry_send.or_else(|_| send_rx.recv_timeout(CHANNEL_TIMEOUT));
				retry_send = Err(());
				match maybe_data {
					Ok(data) => {
						let written =
							try_break!(write_message(&mut writer, &data, writer_tracker.clone()));
						if written.is_none() {
							retry_send = Ok(data);
						}
					}
					Err(RecvTimeoutError::Disconnected) => {
						debug!("peer_write: mpsc channel disconnected during recv_timeout");
						break;
					}
					Err(RecvTimeoutError::Timeout) => {}
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
					.unwrap_or_else(|_| "?".to_owned())
			);
			let _ = writer.shutdown(Shutdown::Both);
		})?;
	Ok((reader_thread, writer_thread))
}
