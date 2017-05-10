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

use core::core::{Block, BlockHeader, Transaction};
use core::core::hash::Hash;

use grin_store::codec::{BlockCodec, TxCodec};

use msg::*;
use msg::MsgHeader;

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

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
		unimplemented!()
	}
}

impl codec::Decoder for MsgCodec {
	type Item = Message;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		unimplemented!()
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