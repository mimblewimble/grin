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

//! Hash Function
//!
//! Primary hash function used in the protocol
//!

use std::{fmt, ops};
use tiny_keccak::Keccak;
use std::convert::AsRef;
use tokio_io::codec::Encoder;

use codec::{HashEncode, block, tx};
use bytes::BytesMut;
use ser::{self, Reader, Readable, Writer, Writeable, Error, AsFixedBytes};

/// A hash consisting of all zeroes, used as a sentinel. No known preimage.
pub const ZERO_HASH: Hash = Hash([0; 32]);

/// A hash to uniquely (or close enough) identify one of the main blockchain
/// constructs. Used pervasively for blocks, transactions and ouputs.
#[derive(Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl fmt::Debug for Hash {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in self.0[..].iter().cloned() {
			try!(write!(f, "{:02x}", i));
		}
		Ok(())
	}
}

impl fmt::Display for Hash {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		fmt::Debug::fmt(self, f)
	}
}

impl Hash {
	/// Converts the hash to a byte vector
	pub fn to_vec(&self) -> Vec<u8> {
		self.0.to_vec()
	}
}

impl ops::Index<usize> for Hash {
	type Output = u8;

	fn index(&self, idx: usize) -> &u8 {
		&self.0[idx]
	}
}

impl ops::Index<ops::Range<usize>> for Hash {
	type Output = [u8];

	fn index(&self, idx: ops::Range<usize>) -> &[u8] {
		&self.0[idx]
	}
}

impl ops::Index<ops::RangeTo<usize>> for Hash {
	type Output = [u8];

	fn index(&self, idx: ops::RangeTo<usize>) -> &[u8] {
		&self.0[idx]
	}
}

impl ops::Index<ops::RangeFrom<usize>> for Hash {
	type Output = [u8];

	fn index(&self, idx: ops::RangeFrom<usize>) -> &[u8] {
		&self.0[idx]
	}
}

impl ops::Index<ops::RangeFull> for Hash {
	type Output = [u8];

	fn index(&self, idx: ops::RangeFull) -> &[u8] {
		&self.0[idx]
	}
}

impl AsRef<[u8]> for Hash {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl Readable for Hash {
	fn read(reader: &mut Reader) -> Result<Hash, ser::Error> {
		let v = try!(reader.read_fixed_bytes(32));
		let mut a = [0; 32];
		for i in 0..a.len() {
			a[i] = v[i];
		}
		Ok(Hash(a))
	}
}

/// Serializer that outputs a hash of the serialized object
pub struct HashWriter {
	state: Keccak,
}

impl HashWriter {
	/// Consume the `HashWriter`, outputting its current hash into a 32-byte
	/// array
	pub fn finalize(self, output: &mut [u8]) {
		self.state.finalize(output);
	}

	/// Consume the `HashWriter`, outputting a `Hash` corresponding to its
	/// current state
	pub fn into_hash(self) -> Hash {
		let mut new_hash = ZERO_HASH;
		self.state.finalize(&mut new_hash.0[..]);
		new_hash
	}
}

impl Default for HashWriter {
	fn default() -> HashWriter {
		HashWriter { state: Keccak::new_sha3_256() }
	}
}

/// A trait for types that have a canonical hash
pub trait Hashed {
	/// Obtain the hash of the object
	fn hash(&self) -> Hash;
}

// impl<T: Writeable> Hashed for T {
// 	fn hash(&self) -> Hash {
// 		let mut hasher = HashWriter::default();
// 		ser::Writeable::write(self, &mut hasher).unwrap();
// 		let mut ret = [0; 32];
// 		hasher.finalize(&mut ret);

// 		Hash(ret)
// 	}
// }

impl<T: HashEncode + Clone> Hashed for T {
	fn hash(&self) -> Hash {
		let mut codec = T::HashEncoder::default();
		let mut hasher = HashWriter::default();

		let mut bytes = BytesMut::with_capacity(0);
		let mut ret = [0; 32];

		codec.encode(self.clone(), &mut bytes);
		hasher.state.update(bytes.as_ref());
		hasher.finalize(&mut ret);

		Hash(ret)
	}
}

impl<A: HashEncode, B: HashEncode> Hashed for (A, B) {
	fn hash(&self) -> Hash {
		let mut codec_a = A::HashEncoder::default();
		let mut codec_b = B::HashEncoder::default();

		let mut hasher = HashWriter::default();

		let mut bytes = BytesMut::with_capacity(0);

		let mut ret = [0; 32];

		codec_a.encode(self.0.clone(), &mut bytes);
		codec_b.encode(self.1.clone(), &mut bytes);

		hasher.state.update(bytes.as_ref());

		hasher.finalize(&mut ret);

		Hash(ret)
	}
}

impl<A: HashEncode, B: HashEncode, C: HashEncode> Hashed for (A, B, C) {
	fn hash(&self) -> Hash {
		let mut codec_a = A::HashEncoder::default();
		let mut codec_b = B::HashEncoder::default();
		let mut codec_c = C::HashEncoder::default();

		let mut hasher = HashWriter::default();

		let mut bytes = BytesMut::with_capacity(0);

		let mut ret = [0; 32];

		codec_a.encode(self.0.clone(), &mut bytes);
		codec_b.encode(self.1.clone(), &mut bytes);
		codec_c.encode(self.2.clone(), &mut bytes);

		hasher.state.update(bytes.as_ref());
		hasher.finalize(&mut ret);

		Hash(ret)
	}
}

// impl Hashed for u8 {}
// impl Hashed for u16 {}
// impl Hashed for u32 {}
// impl Hashed for u64 {}
// impl Hashed for i64 {}

// Convenience for when we need to hash of an empty array.
impl Hashed for [u8; 0] {
	fn hash(&self) -> Hash {
		let hasher = HashWriter::default();
		let mut ret = [0; 32];
		hasher.finalize(&mut ret);
		Hash(ret)
	}
}
