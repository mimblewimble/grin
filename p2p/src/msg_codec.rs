// Copyright 2016 The Grin Developers
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

//! Implementation of the p2p message encoding and decoding.

use std::io;

use tokio_io::*;
use bytes::{BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use tokio_io::codec::{Encoder, Decoder};
use enum_primitive::FromPrimitive;

use core::core::{Block, BlockHeader, Transaction};
use core::core::hash::Hash;

use grin_store::codec::{BlockCodec, TxCodec};

use msg::*;
use msg::MsgHeader;

const MSG_HEADER_SIZE:usize = 11;

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

#[derive(Clone, Debug, PartialEq)]
enum Message {
	Error(PeerError),
	Hand(Hand),
	Shake(Shake),
	Ping,
	Pong,
	GetPeerAddrs(GetPeerAddrs),
	PeerAddrs(PeerAddrs),
	GetHeaders(Locator),
	Headers(Headers),
	GetBlock(Hash),
	Block(Block),
	Transaction(Transaction),
}

/// Codec for Decoding and Encoding a `MsgHeader`
#[derive(Debug, Clone, Default)]
struct MsgCodec;

impl codec::Encoder for MsgCodec {
	type Item = Message;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		dst.reserve(MSG_HEADER_SIZE);

		let msg_dst = BytesMut::with_capacity(0);

		let header = match item {
			Message::Pong => {

				MsgHeader::new(Type::Pong, 0)
			},
			Message::Ping => {
				MsgHeader::new(Type::Ping, 0)
			},
			_ => unimplemented!()
		};

		dst.put_slice(&header.magic);
		dst.put_u8(header.msg_type as u8);
		dst.put_u64::<BigEndian>(header.msg_len);

		dst.reserve(msg_dst.len());
		dst.put_slice(&msg_dst);
		// dst.reserve()

		Ok(())

	}
}

impl codec::Decoder for MsgCodec {
	type Item = Message;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Decode Header
		if src.len() < MSG_HEADER_SIZE {
			println!("returned at header length");
			return Ok(None);
		}
		let mut buf = src.split_to(MSG_HEADER_SIZE).into_buf();
		
		// Get Magic
		let mut some_magic = [0;2];
		buf.copy_to_slice(&mut some_magic);

		// If Magic is invalid return error.
		if some_magic != MAGIC {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Header"));
		}

		let msg_type = match Type::from_u8(buf.get_u8()) {
			Some(t) => t,
			None => { 
				return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Message Type"));
			},
		}; 

		let msg_len = buf.get_u64::<BigEndian>() as usize;
		if src.len() < msg_len {
			println!("returned at msg length");
			return Ok(None);
		}

		let decoded_msg = match msg_type {
			Type::Ping => Message::Ping,
			Type::Pong => Message::Pong,
			_ => unimplemented!()
		};

		Ok(Some(decoded_msg))
	}
}

// Internal Convenience Trait
trait MsgEncode: Sized {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Internal Convenience Trait
trait MsgDecode: Sized {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

// impl MsgEncode for Pong {
// 	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
// 		Ok()
// 	}
// }


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn should_encode_decode_ping() {
		let mut codec = MsgCodec;
		let ping = Message::Ping;
		let mut buf = BytesMut::with_capacity(0);

		codec.encode(ping.clone(), &mut buf).expect("Expected to encode ping");
		let result = codec.decode(&mut buf).expect("Expected no Errors to decode ping").unwrap();
		assert_eq!(ping, result);
	}

	#[test]
	fn should_decode_encode_pong() {
		let mut codec = MsgCodec;
		let pong = Message::Pong;
		let mut buf = BytesMut::with_capacity(0);

		codec.encode(pong.clone(), &mut buf).expect("Expected to encode pong");
		let result = codec.decode(&mut buf).expect("Expected no Errors to decode pong").unwrap();
		assert_eq!(pong, result);
	}
}
