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
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6, Ipv4Addr, Ipv6Addr, IpAddr};

use tokio_io::*;
use bytes::{BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use tokio_io::codec::{Encoder, Decoder};
use enum_primitive::FromPrimitive;

use core::core::{Block, BlockHeader, Input, Output, Transaction, TxKernel};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::transaction::{OutputFeatures, KernelFeatures};
use types::*;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

use grin_store::codec::{BlockCodec, TxCodec};

use msg::*;
use msg::MsgHeader;

const MSG_HEADER_SIZE: usize = 11;
const SOCKET_ADDR_MARKER_V4: u8 = 0;
const SOCKET_ADDR_MARKER_V6: u8 = 1;

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

#[derive(Clone, Debug, PartialEq)]
enum Message {
	PeerError(PeerError),
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

		let mut msg_dst = BytesMut::with_capacity(0);

		let header = match item {
			Message::Pong => MsgHeader::new(Type::Pong, 0),
			Message::Ping => MsgHeader::new(Type::Ping, 0),
			Message::Hand(hand) => {
				hand.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::Hand, msg_dst.len() as u64)
			}
			Message::Shake(shake) => {
				shake.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::Shake, msg_dst.len() as u64)
			}
			Message::GetPeerAddrs(get_peer_addrs) => {
				get_peer_addrs.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::GetPeerAddrs, msg_dst.len() as u64)
			}
			Message::PeerAddrs(peer_addrs) => {
				peer_addrs.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::PeerAddrs, msg_dst.len() as u64)
			}
			Message::Headers(headers) => {
				headers.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::Headers, msg_dst.len() as u64)
			}
			Message::GetHeaders(locator) => {
				locator.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::GetHeaders, msg_dst.len() as u64)
			}
			Message::Block(block) => {
				let mut codec = BlockCodec::default();
				codec.encode(block, &mut msg_dst)?;
				MsgHeader::new(Type::Block, msg_dst.len() as u64)
			}
			Message::GetBlock(hash) => {
				let mut codec = BlockCodec::default();
				codec.encode(hash, &mut msg_dst)?;
				MsgHeader::new(Type::GetBlock, msg_dst.len() as u64)
			}
			Message::Transaction(tx) => {
				let mut codec = TxCodec::default();
				codec.encode(tx, &mut msg_dst)?;
				MsgHeader::new(Type::Transaction, msg_dst.len() as u64)
			}
			Message::PeerError(err) => {
				err.msg_encode(&mut msg_dst)?;
				MsgHeader::new(Type::Error, msg_dst.len() as u64)
			}
		};

		// Put MsgHeader
		dst.put_slice(&header.magic);
		dst.put_u8(header.msg_type as u8);
		dst.put_u64::<BigEndian>(header.msg_len);

		// Put Body
		dst.reserve(msg_dst.len());
		dst.put_slice(&msg_dst);

		Ok(())

	}
}

impl codec::Decoder for MsgCodec {
	type Item = Message;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Create Temporary Buffer
		let ref mut temp_src = src.clone();
		
		// Decode Header
		if temp_src.len() < MSG_HEADER_SIZE {
			return Ok(None);
		}
		let mut buf = temp_src.split_to(MSG_HEADER_SIZE).into_buf();

		// Get Magic
		let mut some_magic = [0; 2];
		buf.copy_to_slice(&mut some_magic);

		// If Magic is invalid return error.
		if some_magic != MAGIC {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Header"));
		}

		// Ensure Valid Message Type
		let msg_type = match Type::from_u8(buf.get_u8()) {
			Some(t) => t,
			None => {
				return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Message Type"));
			}
		};

		// Ensure sufficient data
		let msg_len = buf.get_u64::<BigEndian>() as usize;
		if temp_src.len() < msg_len {
			return Ok(None);
		}

		// Attempt message body decode
		let decoded_msg = match msg_type {
			Type::Ping => Message::Ping,
			Type::Pong => Message::Pong,
			Type::Hand => {
				let hand = try_opt_dec!(Hand::msg_decode(temp_src)?);
				Message::Hand(hand)
			}
			Type::Shake => {
				let shake = try_opt_dec!(Shake::msg_decode(temp_src)?);
				Message::Shake(shake)
			}
			Type::GetPeerAddrs => {
				let get_peer_addrs = try_opt_dec!(GetPeerAddrs::msg_decode(temp_src)?);
				Message::GetPeerAddrs(get_peer_addrs)
			}
			Type::PeerAddrs => {
				let peer_addrs = try_opt_dec!(PeerAddrs::msg_decode(temp_src)?);
				Message::PeerAddrs(peer_addrs)
			}
			Type::Headers => {
				let headers = try_opt_dec!(Headers::msg_decode(temp_src)?);
				Message::Headers(headers)
			}
			Type::GetHeaders => {
				let locator = try_opt_dec!(Locator::msg_decode(temp_src)?);
				Message::GetHeaders(locator)
			}
			Type::Block => {
				let mut codec = BlockCodec::default();
				let block = try_opt_dec!(codec.decode(temp_src)?); 
				Message::Block(block)
			}
			Type::GetBlock => {
				let mut codec = BlockCodec::default();
				let hash = try_opt_dec!(codec.decode(temp_src)?); 
				Message::GetBlock(hash)
			}
			Type::Transaction => {
				let mut codec = TxCodec::default();
				let transaction = try_opt_dec!(codec.decode(temp_src)?); 
				Message::Transaction(transaction)
			}
			Type::Error => {
				let err = try_opt_dec!(PeerError::msg_decode(temp_src)?);
				Message::PeerError(err)
			}
		};

		// If succesfull truncate src by bytes read from temp_src;
		let diff = src.len() - temp_src.len();
		src.split_to(diff);

		Ok(Some(decoded_msg))
	}
}

// Internal Convenience Traits
pub trait MsgEncode: Sized {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

// Internal Convenience Trait
pub trait MsgDecode: Sized {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

impl MsgEncode for Hand {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		// Reserve for version(4), capabilities(4), nonce(8)
		dst.reserve(16);
		// Put Protocol Version
		dst.put_u32::<BigEndian>(self.version);
		// Put Capabilities
		dst.put_u32::<BigEndian>(self.capabilities.bits());
		// Put Nonce
		dst.put_u64::<BigEndian>(self.nonce);

		// Put Difficulty with BlockCodec
		BlockCodec::default()
			.encode(self.total_difficulty.clone(), dst)?;

		// Put Sender Address
		self.sender_addr.0.msg_encode(dst)?;
		// Put Receier Address
		self.receiver_addr.0.msg_encode(dst)?;

		// Put Size of String
		let str_bytes = self.user_agent.as_bytes();
		dst.reserve(str_bytes.len() + 1);

		// Put Software Version
		dst.put_u8(str_bytes.len() as u8);
		dst.put_slice(str_bytes);

		Ok(())
	}
}

impl MsgDecode for Hand {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		// TODO: Check for Full Hand Size Upfront
		if src.len() < 16 {
			return Ok(None);
		}
		// Get Protocol Version, Capabilities, Nonce
		let mut buf = src.split_to(16).into_buf();
		let version = buf.get_u32::<BigEndian>();
		let capabilities = Capabilities::from_bits(buf.get_u32::<BigEndian>()).unwrap_or(UNKNOWN);
		let nonce = buf.get_u64::<BigEndian>();

		// Get Total Difficulty
		let total_difficulty = try_opt_dec!(BlockCodec::default().decode(src)?);

		// Get Sender and Receiver Addresses
		let sender_addr = try_opt_dec!(SocketAddr::msg_decode(src)?);
		let receiver_addr = try_opt_dec!(SocketAddr::msg_decode(src)?);


		// Get Software Version
		// TODO: Decide on Hand#user_agent size
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let str_len = buf.get_u8() as usize;
		if src.len() < str_len {
			return Ok(None);
		}
		let buf = src.split_to(str_len).into_buf();
		let user_agent = String::from_utf8(buf.collect())
			.map_err(|_| {
				         io::Error::new(io::ErrorKind::InvalidData, "Invalid Hand Software Version")
				        })?;

		Ok(Some(Hand {
		            version: version,
		            capabilities: capabilities,
		            nonce: nonce,
		            total_difficulty: total_difficulty,
		            sender_addr: SockAddr(sender_addr),
		            receiver_addr: SockAddr(receiver_addr),
		            user_agent: user_agent,
		        }))

	}
}

impl MsgEncode for Shake {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		// Reserve for version(4), capabilities(4)
		dst.reserve(8);
		// Put Protocol Version
		dst.put_u32::<BigEndian>(self.version);
		// Put Capabilities
		dst.put_u32::<BigEndian>(self.capabilities.bits());

		// Put Difficulty with BlockCodec
		BlockCodec::default()
			.encode(self.total_difficulty.clone(), dst)?;

		// Reserve for user agent string Size of String
		let str_bytes = self.user_agent.as_bytes();
		dst.reserve(str_bytes.len() + 1);

		// Put user agent string
		dst.put_u8(str_bytes.len() as u8);
		dst.put_slice(str_bytes);

		Ok(())
	}
}

impl MsgDecode for Shake {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		// TODO: Check for Full Hand Size Upfront
		if src.len() < 8 {
			return Ok(None);
		}
		// Get Protocol Version, Capabilities, Nonce
		let mut buf = src.split_to(8).into_buf();
		let version = buf.get_u32::<BigEndian>();
		let capabilities = Capabilities::from_bits(buf.get_u32::<BigEndian>()).unwrap_or(UNKNOWN);

		// Get Total Difficulty
		let total_difficulty = try_opt_dec!(BlockCodec::default().decode(src)?);

		// Get Software Version
		// TODO: Decide on Hand#user_agent size
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let str_len = buf.get_u8() as usize;
		if src.len() < str_len {
			return Ok(None);
		}
		let buf = src.split_to(str_len).into_buf();
		let user_agent = String::from_utf8(buf.collect())
			.map_err(|_| {
				         io::Error::new(io::ErrorKind::InvalidData, "Invalid Hand Software Version")
				        })?;

		Ok(Some(Shake {
		            version: version,
		            capabilities: capabilities,
		            total_difficulty: total_difficulty,
		            user_agent: user_agent,
		        }))

	}
}

impl MsgEncode for GetPeerAddrs {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		// Reserve for and put Capabilities
		dst.reserve(4);
		dst.put_u32::<BigEndian>(self.capabilities.bits());
		Ok(())
	}
}

impl MsgDecode for GetPeerAddrs {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 4 {
			return Ok(None);
		}
		let mut buf = src.split_to(4).into_buf();
		let capabilities = Capabilities::from_bits(buf.get_u32::<BigEndian>()).unwrap_or(UNKNOWN);
		Ok(Some(GetPeerAddrs { capabilities: capabilities }))
	}
}

impl MsgEncode for PeerAddrs {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(4);
		dst.put_u32::<BigEndian>(self.peers.len() as u32);
		for p in &self.peers {
			p.0.msg_encode(dst)?;
		}
		Ok(())
	}
}

impl MsgDecode for PeerAddrs {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 4 {
			return Ok(None);
		}
		let mut buf = src.split_to(4).into_buf();
		let peer_count = buf.get_u32::<BigEndian>();
		// Check peer count is valid, return error or empty if applicable
		if peer_count > MAX_PEER_ADDRS {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Peer Count"));
		} else if peer_count == 0 {
			return Ok(Some(PeerAddrs { peers: vec![] }));
		}

		let mut peers = Vec::with_capacity(peer_count as usize);
		for _ in 0..peer_count {
			let p = SocketAddr::msg_decode(src)?;
			// XXX: Do not need SockAddr wrapper??
			peers.push(SockAddr(p.unwrap()));
		}

		Ok(Some(PeerAddrs { peers: peers }))
	}
}

impl MsgEncode for Headers {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(2);
		dst.put_u16::<BigEndian>(self.headers.len() as u16);
		let mut block_codec = BlockCodec::default();
		for h in &self.headers {
			block_codec.encode(h.clone(), dst)?;
		}

		Ok(())
	}
}

impl MsgDecode for Headers {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 2 {
			return Ok(None);
		}
		// Get Headers Length
		let mut buf = src.split_to(2).into_buf();
		let len = buf.get_u16::<BigEndian>();

		// Collect Headers
		let mut headers = Vec::with_capacity(len as usize);
		let mut block_codec = BlockCodec::default();
		for _ in 0..len {
			let block = try_opt_dec!(block_codec.decode(src)?);
			headers.push(block);
		}
		Ok(Some(Headers { headers: headers }))
	}
}

impl MsgEncode for Locator {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result< (), io::Error > {
		dst.reserve(1);
		let len = self.hashes.len() as u8;
		dst.put_u8(len);

		let mut block_codec = BlockCodec::default();
		for h in &self.hashes {
			block_codec.encode(h.clone(), dst)?;
		}
		Ok(())
	}
}

impl MsgDecode for Locator {
	fn msg_decode(src: &mut BytesMut) -> Result< Option<Self>, io::Error > {
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let len = buf.get_u8();

		let mut hashes = Vec::with_capacity(len as usize);
		let mut block_codec = BlockCodec::default();
		for _ in 0..len {
			let hash = try_opt_dec!(block_codec.decode(src)?);
			hashes.push(hash);
		}

		Ok(Some(Locator { hashes: hashes }))
	}
}

impl MsgEncode for PeerError {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result< (), io::Error > {
		dst.reserve(4);
		dst.put_u32::<BigEndian>(self.code);
		
		let bytes = self.message.as_bytes();
		dst.reserve(1 + bytes.len());
		dst.put_u8(bytes.len() as u8);
		dst.put_slice(bytes);
		Ok(())
	}
}


impl MsgDecode for PeerError {
	fn msg_decode(src: &mut BytesMut) -> Result< Option<Self>, io::Error > {
		// Reserve for code(4) and msg length(1)
		if src.len() < 5 {
			return Ok(None);
		}
		let mut buf = src.split_to(5).into_buf();
		let code = buf.get_u32::<BigEndian>();
		
		let str_len = buf.get_u8() as usize;

		if src.len() < str_len {
			return Ok(None);
		}

		let buf = src.split_to(str_len).into_buf();

		let message = String::from_utf8(buf.collect())
			.map_err(|_| {
				         io::Error::new(io::ErrorKind::InvalidData, "Invalid Error Message Field")
				        })?;

		Ok(Some( PeerError {
			code: code,
			message: message
		}))
	}
}

impl MsgEncode for SocketAddr {
	fn msg_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		match *self {
			SocketAddr::V4(sav4) => {
				dst.reserve(7);
				dst.put_u8(SOCKET_ADDR_MARKER_V4);
				dst.put_slice(&sav4.ip().octets());
				dst.put_u16::<BigEndian>(sav4.port());
				Ok(())
			}
			SocketAddr::V6(sav6) => {
				dst.reserve(19);
				dst.put_u8(SOCKET_ADDR_MARKER_V6);

				for seg in &sav6.ip().segments() {
					dst.put_u16::<BigEndian>(*seg);
				}

				dst.put_u16::<BigEndian>(sav6.port());
				Ok(())
			}
		}
	}
}

impl MsgDecode for SocketAddr {
	fn msg_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 7 {
			return Ok(None);
		}

		let marker = src.split_to(1)[0];
		match marker {
			SOCKET_ADDR_MARKER_V4 => {
				let mut buf = src.split_to(6).into_buf();

				// Get V4 address
				let mut ip = [0; 4];
				buf.copy_to_slice(&mut ip);

				// Get port
				let port = buf.get_u16::<BigEndian>();

				// Build v4 socket
				let socket = SocketAddrV4::new(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), port);
				Ok(Some(SocketAddr::V4(socket)))
			}
			SOCKET_ADDR_MARKER_V6 => {
				if src.len() < 18 {
					return Ok(None);
				}
				let mut buf = src.split_to(18).into_buf();

				// Get V6 address
				let mut ip = [0u16; 8];
				for i in 0..8 {
					ip[i] = buf.get_u16::<BigEndian>();
				}

				// Get Port
				let port = buf.get_u16::<BigEndian>();

				// Build V6 socket
				let socket = SocketAddrV6::new(Ipv6Addr::new(ip[0],
				                                             ip[1],
				                                             ip[2],
				                                             ip[3],
				                                             ip[4],
				                                             ip[5],
				                                             ip[6],
				                                             ip[7]),
				                               port,
				                               0,
				                               0);

				Ok(Some(SocketAddr::V6(socket)))
			}
			_ => Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Socket Marker")),
		}
	}
}



#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn should_encode_decode_ping() {
		let mut codec = MsgCodec;
		let ping = Message::Ping;
		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(ping.clone(), &mut buf)
			.expect("Expected to encode ping message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode ping message")
			.unwrap();


		assert_eq!(ping, result);
	}
	
	fn should_handle_incomplete_without_modifying_src_ping() {
		let mut codec = MsgCodec;
		let ping = Message::Ping;
		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(ping.clone(), &mut buf)
			.expect("Expected to encode ping message");
				
		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode ping message")
			.unwrap();
		assert_eq!(ping, result);
	}
	

	#[test]
	fn should_encode_decode_pong() {
		let mut codec = MsgCodec;
		let pong = Message::Pong;
		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(pong.clone(), &mut buf)
			.expect("Expected to encode pong message");
		
		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode pong message")
			.unwrap();
		assert_eq!(pong, result);
	}

	#[test]
	fn should_encode_decode_hand() {
		let mut codec = MsgCodec;
		let sample_socket_addr = SockAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		                                                  8080));
		let hand = Message::Hand(Hand {
		                             version: 0,
		                             capabilities: UNKNOWN,
		                             nonce: 0,
		                             total_difficulty: Difficulty::one(),
		                             sender_addr: sample_socket_addr.clone(),
		                             receiver_addr: sample_socket_addr.clone(),
		                             user_agent: "test".to_string(),
		                         });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(hand.clone(), &mut buf)
			.expect("Expected to encode hand message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode hand message")
			.expect("Expected a full hand message");

		assert_eq!(hand, result);
	}

	#[test]
	fn should_encode_decode_shake() {
		let mut codec = MsgCodec;
		let shake = Message::Shake(Shake {
		                               version: 0,
		                               capabilities: UNKNOWN,
		                               total_difficulty: Difficulty::one(),
		                               user_agent: "test".to_string(),
		                           });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(shake.clone(), &mut buf)
			.expect("Expected to encode shake message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode shake message")
			.expect("Expected full shake message.");

		assert_eq!(shake, result);
	}

	#[test]
	fn should_encode_decode_get_peer_addrs() {
		let mut codec = MsgCodec;
		let get_peer_addrs = Message::GetPeerAddrs(GetPeerAddrs { capabilities: UNKNOWN });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(get_peer_addrs.clone(), &mut buf)
			.expect("Expected to encode get peer addrs message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode get peer addrs message")
			.expect("Expected full get peer addrs message.");

		assert_eq!(get_peer_addrs, result);
	}

	#[test]
	fn should_encode_decode_peer_addrs() {
		let mut codec = MsgCodec;
		let sample_socket_addr = SockAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		                                                  8000));
		let peer_addrs = Message::PeerAddrs(PeerAddrs { peers: vec![sample_socket_addr] });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(peer_addrs.clone(), &mut buf)
			.expect("Expected to encode peer addrs message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode peer addrs message")
			.expect("Expected full peer addrs message");

		assert_eq!(peer_addrs, result);
	}

	#[test]
	fn should_encode_decode_headers() {
		let mut codec = MsgCodec;
		let sample_socket_addr = SockAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		                                                  8000));

		let headers = Message::Headers(Headers { headers: vec![BlockHeader::default()] });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(headers.clone(), &mut buf)
			.expect("Expected to encode headers message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode headers message")
			.expect("Expected full headers message");

		assert_eq!(headers, result);
	}

	#[test]
	fn should_encode_decode_get_headers() {
		let mut codec = MsgCodec;
		let sample_socket_addr = SockAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		                                                  8000));

		let get_headers = Message::GetHeaders(Locator { hashes: vec![Hash([1; 32])] });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(get_headers.clone(), &mut buf)
			.expect("Expected to encode get headers msg");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode get headers msg")
			.unwrap();

		assert_eq!(get_headers, result);
	}

	#[test]
	fn should_encode_decode_get_block() {
		let mut codec = MsgCodec;

		let get_block = Message::GetBlock(Hash([1; 32]));

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(get_block.clone(), &mut buf)
			.expect("Expected to encode hand");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode hand")
			.unwrap();

		assert_eq!(get_block, result);
	}

	#[test]
	fn should_encode_decode_block() {
		let mut codec = MsgCodec;

		let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));
		let output = Output {
			features: OutputFeatures::empty(),
			commit: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
			proof: RangeProof {
				proof: [1; 5134],
				plen: 5134,
			},
		};

		let kernel = TxKernel {
			features: KernelFeatures::empty(),
			excess: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
			excess_sig: vec![1; 10],
			fee: 100,
		};

		let new_block = Block {
			header: BlockHeader::default(),
			inputs: vec![input],
			outputs: vec![output],
			kernels: vec![kernel],
		};

		let block = Message::Block(new_block);
		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(block.clone(), &mut buf)
			.expect("Expected to encode");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode")
			.unwrap();

		assert_eq!(block, result);
	}

	#[test]
	fn should_encode_decode_transaction() {
		let mut codec = MsgCodec;
		let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));
		let output = Output {
			features: OutputFeatures::empty(),
			commit: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
			proof: RangeProof {
				proof: [1; 5134],
				plen: 5134,
			},
		};

		let transaction = Message::Transaction(Transaction {
		                                           inputs: vec![input],
		                                           outputs: vec![output],
		                                           fee: 1 as u64,
		                                           excess_sig: vec![0; 10],
		                                       });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(transaction.clone(), &mut buf)
			.expect("Expected to encode transaction message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode transaction message")
			.unwrap();

		assert_eq!(transaction, result);
	}

	#[test]
	fn should_encode_decode_error() {
		let mut codec = MsgCodec;

		let error = Message::PeerError(PeerError {
		                               code: 0,
		                               message: "Uhoh".to_owned(),
		                           });

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(error.clone(), &mut buf)
			.expect("Expected to encode error message");
		
		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode error message")
			.unwrap();

		assert_eq!(error, result);
	}
}
