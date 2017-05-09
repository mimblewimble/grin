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

//! Implementation of the chain block encoding and decoding.

use std::io;

use tokio_io::*;
use bytes::{BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use num_bigint::BigUint;

use types::Tip;
use core::core::hash::Hash;
use core::core::target::Difficulty;

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

/// Codec for Decoding and Encoding a `Tip`
#[derive(Debug, Clone, Default)]
pub struct ChainCodec;

impl codec::Encoder for ChainCodec {
	type Item = Tip;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		// Put Height
		dst.reserve(8);
		dst.put_u64::<BigEndian>(item.height);

		// Put Last Block Hash
		item.last_block_h.chain_encode(dst)?;

		// Put Previous Block Hash
		item.prev_block_h.chain_encode(dst)?;

		// Put Difficulty
		item.total_difficulty.chain_encode(dst)?;

		Ok(())
	}
}

impl codec::Decoder for ChainCodec {
	type Item = Tip;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {

		// Create Temporary Buffer
		let ref mut temp = src.clone();

		// Get Height
		if temp.len() < 8 {
			return Ok(None);
		}
		let mut buf = temp.split_to(8).into_buf();
		let height = buf.get_u64::<BigEndian>();

		// Get Last Block Hash
		let last_block_h = try_opt_dec!(Hash::chain_decode(temp)?);

		// Get Previous Block Hash
		let prev_block_h = try_opt_dec!(Hash::chain_decode(temp)?);

		// Get Difficulty
		let total_difficulty = try_opt_dec!(Difficulty::chain_decode(temp)?);

		// If succesfull truncate src by bytes read from temp;
		let diff = src.len() - temp.len();
		src.split_to(diff);

		Ok(Some(Tip {
			height: height,
			last_block_h: last_block_h,
			prev_block_h: prev_block_h,
			total_difficulty: total_difficulty,
		}))
	}
}

/// Internal Convenience Trait
trait ChainEncode: Sized {
	fn chain_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Internal Convenience Trait
trait ChainDecode: Sized {
	fn chain_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

impl ChainEncode for Difficulty {
	fn chain_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		let data = self.clone().into_biguint().to_bytes_be();
		dst.reserve(1 + data.len());
		dst.put_u8(data.len() as u8);
		dst.put_slice(&data);
		Ok(())
	}
}

impl ChainDecode for Difficulty {
	fn chain_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let dlen = buf.get_u8() as usize;

		if src.len() < dlen {
			return Ok(None);
		}

		let buf = src.split_to(dlen).into_buf();
		let data = Buf::bytes(&buf);

		Ok(Some(Difficulty::from_biguint(BigUint::from_bytes_be(data))))
	}
}

impl ChainEncode for Hash {
	fn chain_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(32);
		dst.put_slice(self.as_ref());
		Ok(())
	}
}

impl ChainDecode for Hash {
	fn chain_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 32 {
			return Ok(None);
		}

		let mut buf = src.split_to(32).into_buf();
		let mut hash_data = [0; 32];
		buf.copy_to_slice(&mut hash_data);

		Ok(Some(Hash(hash_data)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn should_have_chain_codec_roundtrip() {
		use tokio_io::codec::{Encoder, Decoder};

		let sample_gdb = Hash([1u8; 32]);
		let tip = Tip::new(sample_gdb);

		let mut buf = BytesMut::with_capacity(0);
		let mut codec = ChainCodec {};
		codec.encode(tip.clone(), &mut buf).expect("Error During Tip Encoding");

		let d_tip =
			codec.decode(&mut buf).expect("Error During Tip Decoding").expect("Unfinished Tip");

		// Check if all bytes are read
		assert_eq!(buf.len(), 0);

		assert_eq!(tip.height, d_tip.height);
		assert_eq!(tip.last_block_h, d_tip.last_block_h);
		assert_eq!(tip.prev_block_h, d_tip.prev_block_h);
		assert_eq!(tip.total_difficulty, d_tip.total_difficulty);
	}
}