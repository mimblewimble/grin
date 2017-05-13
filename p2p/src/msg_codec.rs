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

use core::core::{Block, BlockHeader,Input, Output, Transaction, TxKernel};
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
			Message::Pong => MsgHeader::new(Type::Pong, 0),
			Message::Ping => MsgHeader::new(Type::Ping, 0),
			_ => unimplemented!(),
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
		let mut some_magic = [0; 2];
		buf.copy_to_slice(&mut some_magic);

		// If Magic is invalid return error.
		if some_magic != MAGIC {
			return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Header"));
		}

		let msg_type = match Type::from_u8(buf.get_u8()) {
			Some(t) => t,
			None => {
				return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid Message Type"));
			}
		};

		let msg_len = buf.get_u64::<BigEndian>() as usize;
		if src.len() < msg_len {
			println!("returned at msg length");
			return Ok(None);
		}

		let decoded_msg = match msg_type {
			Type::Ping => Message::Ping,
			Type::Pong => Message::Pong,
			_ => unimplemented!(),
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
			.unwrap();

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
			.unwrap();

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
			.unwrap();

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
			.unwrap();

		assert_eq!(peer_addrs, result);
	}

	#[test]
	fn should_encode_decode_headers() {
		let mut codec = MsgCodec;
		let sample_socket_addr = SockAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		                                                  8000));

		let headers = Message::Headers(Headers { 
			headers: vec![BlockHeader::default()] 
		});

		let mut buf = BytesMut::with_capacity(0);

		codec
			.encode(headers.clone(), &mut buf)
			.expect("Expected to encode headers message");

		let result = codec
			.decode(&mut buf)
			.expect("Expected no Errors to decode headers message")
			.unwrap();

		assert_eq!(headers, result);
	}

	#[test]
	fn should_encode_decode_get_headers() {
		let mut codec = MsgCodec;
		let sample_socket_addr = SockAddr(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		                                                  8000));

		let get_headers = Message::GetHeaders(Locator { 
			hashes: vec![Hash([1; 32])],
		});

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

		let transaction = Message::Transaction( Transaction {
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

		let error = Message::Error(PeerError {
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
