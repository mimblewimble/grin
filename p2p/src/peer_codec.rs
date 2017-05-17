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

//! Implementation of the peer data encoding and decoding

use std::io;
use std::net::{SocketAddr, Ipv4Addr, Ipv6Addr, IpAddr};

use tokio_io::*;
use bytes::{BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use tokio_io::codec::{Encoder, Decoder};
use enum_primitive::FromPrimitive;

use types::*;
use msg_codec::{MsgDecode, MsgEncode};
use store::{State, PeerData};

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

/// Codec for Decoding and Encoding a `PeerData`
#[derive(Debug, Clone, Default)]
pub struct PeerCodec;

impl codec::Encoder for PeerCodec {
	type Item = PeerData;
	type Error = io::Error;

	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		// Put socket address as u32
		MsgEncode::msg_encode(&item.addr, dst)?;

		// Put capabilities
		dst.reserve(4);
		dst.put_u32::<BigEndian>(item.capabilities.bits());

		// Put user agent string with u8 length first
		let str_bytes = item.user_agent.as_bytes();
		dst.reserve(str_bytes.len() + 1);
		dst.put_u8(str_bytes.len() as u8);
		dst.put_slice(str_bytes);

		// Put flags
		dst.reserve(1);
		dst.put_u8(item.flags as u8);

		Ok(())
	}
}

impl codec::Decoder for PeerCodec {
	type Item = PeerData;
	type Error = io::Error;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Create Temporary Buffer
		let ref mut temp_src = src.clone();
		
		// Get socket address
		let addr = try_opt_dec!(SocketAddr::msg_decode(temp_src)?);

		// Check for capabilites flags(4), user agent header(1)
		if temp_src.len() < 5 {
			return Ok(None);
		}

		// Get capabilites
		let mut buf = temp_src.split_to(5).into_buf();
		let capabilities = Capabilities::from_bits(buf.get_u32::<BigEndian>()).unwrap_or(UNKNOWN);

		// Check for user agent length(str_len) and flags(1)
		let str_len = buf.get_u8() as usize;
		if temp_src.len() < str_len + 1 {
			return Ok(None);
		}

		// Get User Agent
		let buf = temp_src.split_to(str_len).into_buf();
		let user_agent = String::from_utf8(buf.collect())
			.map_err(|_| {
				         io::Error::new(io::ErrorKind::InvalidData, "Invalid Hand Software Version")
				        })?;

		// Get flags
		let mut buf = temp_src.split_to(1).into_buf();
		let flags_data = buf.get_u8();
		let flags = State::from_u8(flags_data)
			.ok_or(io::Error::new(io::ErrorKind::InvalidData, "Invalid Hand Software Version"))?;

		// If succesfull truncate src by bytes read from temp_src;
		let diff = src.len() - temp_src.len();
		src.split_to(diff);

		Ok(Some(PeerData {
		            addr: addr,
		            capabilities: capabilities,
		            user_agent: user_agent,
		            flags: flags,
		        }))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn should_encode_decode_peer_data() {
		let mut codec = PeerCodec;
		let peer_data = PeerData {
			addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8000),
			capabilities: UNKNOWN,
			user_agent: "foo".to_string(),
			flags: State::Healthy,
		};
		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(peer_data.clone(), &mut buf)
			.expect("Expected to encode peer data message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode peer data message")
			.unwrap();

		assert_eq!(peer_data, result);
	}
}