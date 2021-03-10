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

use crate::core::global::header_size_bytes;
use crate::core::ser::{BufReader, ProtocolVersion, Readable};
use crate::msg::{Message, MsgHeader, MsgHeaderWrapper, Type};
use crate::types::{AttachmentMeta, AttachmentUpdate, Error};
use crate::{
	core::core::block::{BlockHeader, UntrustedBlockHeader},
	msg::HeadersData,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use core::ser::Reader;
use std::cmp::min;
use std::io::Read;
use std::mem;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
use MsgHeaderWrapper::*;
use State::*;

const HEADER_IO_TIMEOUT: Duration = Duration::from_millis(2000);
pub const BODY_IO_TIMEOUT: Duration = Duration::from_millis(60000);
const HEADER_BATCH_SIZE: usize = 32;

enum State {
	None,
	Header(MsgHeaderWrapper),
	BlockHeaders {
		bytes_left: usize,
		items_left: usize,
		headers: Vec<BlockHeader>,
	},
	Attachment(usize, Arc<AttachmentMeta>, Instant),
}

impl State {
	fn is_none(&self) -> bool {
		match self {
			State::None => true,
			_ => false,
		}
	}
}

pub struct Codec {
	pub version: ProtocolVersion,
	stream: TcpStream,
	buffer: BytesMut,
	state: State,
	bytes_read: usize,
}

impl Codec {
	pub fn new(version: ProtocolVersion, stream: TcpStream) -> Self {
		Self {
			version,
			stream,
			buffer: BytesMut::with_capacity(8 * 1024),
			state: None,
			bytes_read: 0,
		}
	}

	/// Destroy the codec and return the reader
	pub fn stream(self) -> TcpStream {
		self.stream
	}

	/// Inform codec next `len` bytes are an attachment
	/// Panics if already reading a body
	pub fn expect_attachment(&mut self, meta: Arc<AttachmentMeta>) {
		assert!(self.state.is_none());
		self.state = Attachment(meta.size, meta, Instant::now());
	}

	/// Length of the next item we are expecting, could be msg header, body, block header or attachment chunk
	fn next_len(&self) -> usize {
		match &self.state {
			None => MsgHeader::LEN,
			Header(Known(h)) if h.msg_type == Type::Headers => {
				// If we are receiving a list of headers, read off the item count first
				min(h.msg_len as usize, 2)
			}
			Header(Known(header)) => header.msg_len as usize,
			Header(Unknown(len, _)) => *len as usize,
			BlockHeaders { bytes_left, .. } => {
				// The header length varies with the number of edge bits. Therefore we overestimate
				// its size and only actually read the bytes we need
				min(*bytes_left, header_size_bytes(63))
			}
			Attachment(left, _, _) => min(*left, 48_000),
		}
	}

	/// Set stream timeout depending on the next expected item
	fn set_stream_timeout(&self) -> Result<(), Error> {
		let timeout = match &self.state {
			None => HEADER_IO_TIMEOUT,
			_ => BODY_IO_TIMEOUT,
		};
		self.stream.set_read_timeout(Some(timeout))?;
		Ok(())
	}

	fn read_inner(&mut self) -> Result<Message, Error> {
		self.bytes_read = 0;
		loop {
			let next_len = self.next_len();
			let pre_len = self.buffer.len();
			// Buffer could already be partially filled, calculate additional bytes we need
			let to_read = next_len.saturating_sub(pre_len);
			if to_read > 0 {
				self.buffer.reserve(to_read);
				for _ in 0..to_read {
					self.buffer.put_u8(0);
				}
				self.set_stream_timeout()?;
				if let Err(e) = self.stream.read_exact(&mut self.buffer[pre_len..]) {
					// Undo reserved bytes on a failed read
					self.buffer.truncate(pre_len);
					return Err(e.into());
				}
				self.bytes_read += to_read;
			}
			match &mut self.state {
				None => {
					// Parse header and keep reading
					let mut raw = self.buffer.split_to(next_len).freeze();
					let mut reader = BufReader::new(&mut raw, self.version);
					let header = MsgHeaderWrapper::read(&mut reader)?;
					self.state = Header(header);
				}
				Header(Known(header)) => {
					let mut raw = self.buffer.split_to(next_len).freeze();
					if header.msg_type == Type::Headers {
						// Special consideration for a list of headers, as we want to verify and process
						// them as they come in instead of only after the full list has been received
						let mut reader = BufReader::new(&mut raw, self.version);
						let items_left = reader.read_u16()? as usize;
						self.state = BlockHeaders {
							bytes_left: header.msg_len as usize - 2,
							items_left,
							headers: Vec::with_capacity(min(HEADER_BATCH_SIZE, items_left)),
						};
					} else {
						// Return full message
						let msg = decode_message(header, &mut raw, self.version);
						self.state = None;
						return msg;
					}
				}
				Header(Unknown(_, msg_type)) => {
					// Discard body and return
					let msg_type = *msg_type;
					self.buffer.advance(next_len);
					self.state = None;
					return Ok(Message::Unknown(msg_type));
				}
				BlockHeaders {
					bytes_left,
					items_left,
					headers,
				} => {
					if *bytes_left == 0 {
						// Incorrect item count
						self.state = None;
						return Err(Error::BadMessage);
					}

					let mut reader = BufReader::new(&mut self.buffer, self.version);
					let header: UntrustedBlockHeader = reader.body()?;
					let bytes_read = reader.bytes_read() as usize;
					headers.push(header.into());
					*bytes_left = bytes_left.saturating_sub(bytes_read);
					*items_left -= 1;
					let remaining = *items_left as u64;
					if headers.len() == HEADER_BATCH_SIZE || remaining == 0 {
						let mut h = Vec::with_capacity(min(HEADER_BATCH_SIZE, *items_left));
						mem::swap(headers, &mut h);
						if remaining == 0 {
							let bytes_left = *bytes_left;
							self.state = None;
							if bytes_left > 0 {
								return Err(Error::BadMessage);
							}
						}
						return Ok(Message::Headers(HeadersData {
							headers: h,
							remaining,
						}));
					}
				}
				Attachment(left, meta, now) => {
					let raw = self.buffer.split_to(next_len).freeze();
					*left -= next_len;
					if now.elapsed().as_secs() > 10 {
						*now = Instant::now();
						debug!("attachment: {}/{}", meta.size - *left, meta.size);
					}
					let update = AttachmentUpdate {
						read: next_len,
						left: *left,
						meta: Arc::clone(meta),
					};
					if *left == 0 {
						self.state = None;
						debug!("attachment: DONE");
					}
					return Ok(Message::Attachment(update, Some(raw)));
				}
			}
		}
	}

	/// Blocking read of the next message
	pub fn read(&mut self) -> (Result<Message, Error>, u64) {
		let msg = self.read_inner();
		(msg, self.bytes_read as u64)
	}
}

// TODO: replace with a macro?
fn decode_message(
	header: &MsgHeader,
	body: &mut Bytes,
	version: ProtocolVersion,
) -> Result<Message, Error> {
	let mut msg = BufReader::new(body, version);
	let c = match header.msg_type {
		Type::Ping => Message::Ping(msg.body()?),
		Type::Pong => Message::Pong(msg.body()?),
		Type::BanReason => Message::BanReason(msg.body()?),
		Type::TransactionKernel => Message::TransactionKernel(msg.body()?),
		Type::GetTransaction => Message::GetTransaction(msg.body()?),
		Type::Transaction => Message::Transaction(msg.body()?),
		Type::StemTransaction => Message::StemTransaction(msg.body()?),
		Type::GetBlock => Message::GetBlock(msg.body()?),
		Type::Block => Message::Block(msg.body()?),
		Type::GetCompactBlock => Message::GetCompactBlock(msg.body()?),
		Type::CompactBlock => Message::CompactBlock(msg.body()?),
		Type::GetHeaders => Message::GetHeaders(msg.body()?),
		Type::Header => Message::Header(msg.body()?),
		Type::GetPeerAddrs => Message::GetPeerAddrs(msg.body()?),
		Type::PeerAddrs => Message::PeerAddrs(msg.body()?),
		Type::TxHashSetRequest => Message::TxHashSetRequest(msg.body()?),
		Type::TxHashSetArchive => Message::TxHashSetArchive(msg.body()?),
		Type::GetOutputBitmapSegment => Message::GetOutputBitmapSegment(msg.body()?),
		Type::OutputBitmapSegment => Message::OutputBitmapSegment(msg.body()?),
		Type::GetOutputSegment => Message::GetOutputSegment(msg.body()?),
		Type::OutputSegment => Message::OutputSegment(msg.body()?),
		Type::GetRangeProofSegment => Message::GetRangeProofSegment(msg.body()?),
		Type::RangeProofSegment => Message::RangeProofSegment(msg.body()?),
		Type::GetKernelSegment => Message::GetKernelSegment(msg.body()?),
		Type::KernelSegment => Message::KernelSegment(msg.body()?),
		Type::Error | Type::Hand | Type::Shake | Type::Headers => {
			return Err(Error::UnexpectedMessage)
		}
	};
	Ok(c)
}
