use crate::core::ser::{self, BufReader, ProtocolVersion, Readable, Reader};
use crate::msg::{Consume, Msg, MsgHeader, MsgHeaderWrapper, MsgWrapper};
use crate::types::{AttachmentMeta, AttachmentUpdate, Error};
use bytes::{BufMut, Bytes, BytesMut};
use chrono::{DateTime, Utc};
use std::cmp::min;
use std::path::PathBuf;
use std::time::Instant;
use tokio_util::codec::{Decoder, Encoder};
use MsgHeaderWrapper::*;
use State::*;

enum State {
	Header(MsgHeaderWrapper),
	Attachment(usize, AttachmentMeta, Instant),
}

pub enum Output {
	Known(MsgHeader, Bytes),
	Unknown(u64, u8),
	Attachment(AttachmentUpdate, Bytes),
}

pub struct Codec {
	pub version: ProtocolVersion,
	state: Option<State>,
}

impl Codec {
	pub fn new(version: ProtocolVersion) -> Self {
		Self {
			version,
			state: None,
		}
	}

	/// Inform codec next `len` bytes are an attachment
	/// Panics if already reading a body
	pub fn expect_attachment(&mut self, meta: AttachmentMeta) {
		assert!(self.state.is_none());
		self.state = Some(Attachment(meta.size, meta, Instant::now()));
	}

	/// Length of the next item we are expecting, could be header, body or attachment chunk
	fn next_len(&self) -> usize {
		match &self.state {
			None => MsgHeader::LEN,
			Some(Header(Known(header))) => header.msg_len as usize,
			Some(Header(Unknown(len, _))) => *len as usize,
			Some(Attachment(left, _, _)) => min(*left, 48_000),
		}
	}
}

impl Decoder for Codec {
	type Item = Output;
	type Error = Error;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		loop {
			let next_len = self.next_len();
			if src.len() >= next_len {
				// We have enough bytes to progress
				let mut raw = src.split_to(next_len).freeze();
				match self.state.take() {
					None => {
						// Parse header and keep reading
						let mut reader = BufReader::new(&mut raw, self.version);
						let header = MsgHeaderWrapper::read(&mut reader)?;
						self.state = Some(Header(header));
					}
					Some(Header(Known(header))) => {
						// Return message
						return Ok(Some(Output::Known(header, raw)));
					}
					Some(Header(Unknown(len, msg_type))) => {
						// Discard body and return
						return Ok(Some(Output::Unknown(len, msg_type)));
					}
					Some(Attachment(mut left, meta, mut now)) => {
						left -= next_len;
						if now.elapsed().as_secs() > 10 {
							now = Instant::now();
							debug!("attachment: {}/{}", meta.size - left, meta.size);
						}
						let update = AttachmentUpdate {
							read: next_len,
							left,
							meta: meta.clone(),
						};
						if left > 0 {
							self.state = Some(Attachment(left, meta, now));
						} else {
							debug!("attachment: DONE");
						}
						return Ok(Some(Output::Attachment(update, raw)));
					}
				}
			} else {
				return Ok(None);
			}
		}
	}
}

impl Encoder for Codec {
	type Item = Bytes;
	type Error = Error;

	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		dst.reserve(item.len());
		dst.put(item);
		Ok(())
	}
}
