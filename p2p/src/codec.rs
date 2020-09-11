use crate::core::ser::{BufReader, ProtocolVersion, Readable};
use crate::msg::{Message, MsgHeader, MsgHeaderWrapper, Type};
use crate::types::{AttachmentMeta, AttachmentUpdate, Error};
use bytes::{BufMut, Bytes, BytesMut};
use std::cmp::min;
use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};
use MsgHeaderWrapper::*;
use State::*;

const HEADER_IO_TIMEOUT: Duration = Duration::from_millis(2000);
pub const BODY_IO_TIMEOUT: Duration = Duration::from_millis(60000);

enum State {
	None,
	Header(MsgHeaderWrapper),
	Attachment(usize, Arc<AttachmentMeta>, Instant),
}

impl State {
	fn take(&mut self) -> Self {
		std::mem::replace(self, State::None)
	}

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

	/// Length of the next item we are expecting, could be header, body or attachment chunk
	fn next_len(&self) -> usize {
		match &self.state {
			None => MsgHeader::LEN,
			Header(Known(header)) => header.msg_len as usize,
			Header(Unknown(len, _)) => *len as usize,
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
			self.buffer.reserve(next_len);
			for _ in 0..next_len {
				self.buffer.put_u8(0);
			}
			let mut buf = self.buffer.split_to(next_len);
			self.set_stream_timeout()?;
			self.stream.read_exact(&mut buf[..])?;
			self.bytes_read += buf.len();
			let mut raw = buf.freeze();
			match self.state.take() {
				None => {
					// Parse header and keep reading
					let mut reader = BufReader::new(&mut raw, self.version);
					let header = MsgHeaderWrapper::read(&mut reader)?;
					self.state = Header(header);
				}
				Header(Known(header)) => {
					// Return message
					return decode_message(&header, &mut raw, self.version);
				}
				Header(Unknown(_, msg_type)) => {
					// Discard body and return
					return Ok(Message::Unknown(msg_type));
				}
				Attachment(mut left, meta, mut now) => {
					left -= next_len;
					if now.elapsed().as_secs() > 10 {
						now = Instant::now();
						debug!("attachment: {}/{}", meta.size - left, meta.size);
					}
					let update = AttachmentUpdate {
						read: next_len,
						left,
						meta: Arc::clone(&meta),
					};
					if left > 0 {
						self.state = Attachment(left, meta, now);
					} else {
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
		Type::Headers => Message::Headers(msg.body()?),
		Type::GetPeerAddrs => Message::GetPeerAddrs(msg.body()?),
		Type::PeerAddrs => Message::PeerAddrs(msg.body()?),
		Type::TxHashSetRequest => Message::TxHashSetRequest(msg.body()?),
		Type::TxHashSetArchive => Message::TxHashSetArchive(msg.body()?),
		Type::Error | Type::Hand | Type::Shake => return Err(Error::UnexpectedMessage),
	};
	Ok(c)
}
