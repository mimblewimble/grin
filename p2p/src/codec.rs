use crate::core::ser::{self, BufReader, FixedLength, ProtocolVersion, Readable, Reader};
use crate::msg::{Msg, MsgHeader, MsgHeaderWrapper, MsgWrapper};
use crate::types::Error;
use bytes::{BufMut, Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};
use MsgHeaderWrapper::*;

pub struct Codec {
	pub version: ProtocolVersion,
	state: Option<MsgHeaderWrapper>,
}

impl Codec {
	pub fn new(version: ProtocolVersion) -> Self {
		Self {
			version,
			state: None,
		}
	}

	/// Length of the next item we are expecting, could either be a header or a body
	fn next_len(&self) -> usize {
		match &self.state {
			None => MsgHeader::LEN,
			Some(Known(header)) => header.msg_len as usize,
			Some(Unknown(len, _)) => *len as usize,
		}
	}
}

impl Decoder for Codec {
	type Item = MsgWrapper;
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
						self.state = Some(header);
					}
					Some(Known(header)) => {
						// Return message
						return Ok(Some(MsgWrapper::Known(Msg::from_bytes(
							header,
							raw,
							self.version,
						))));
					}
					Some(Unknown(len, type_byte)) => {
						// Discard body and keep reading
						return Ok(Some(MsgWrapper::Unknown(len, type_byte)));
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
