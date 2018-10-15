// Copyright 2018 The Grin Developers
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

use byteorder::{BigEndian, ByteOrder};
use std::cmp::min;
use std::convert::AsRef;
use std::ops::Add;
use std::{fmt, ops};

use blake2::blake2b::Blake2b;

use consensus;
use ser::{self, AsFixedBytes, Error, Readable, Reader, Writeable, Writer};
use util;

/// A hash consisting of all zeroes, used as a sentinel. No known preimage.
pub const ZERO_HASH: Hash = Hash([0; 32]);

/// A hash to uniquely (or close enough) identify one of the main blockchain
/// constructs. Used pervasively for blocks, transactions and outputs.
#[derive(Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl fmt::Debug for Hash {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in self.0[..4].iter() {
			write!(f, "{:02x}", i)?;
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
	pub const SIZE: usize = 32;

	/// Builds a Hash from a byte vector. If the vector is too short, it will be
	/// completed by zeroes. If it's too long, it will be truncated.
	pub fn from_vec(v: &[u8]) -> Hash {
		let mut h = [0; Hash::SIZE];
		let copy_size = min(v.len(), Hash::SIZE);
		h[..copy_size].copy_from_slice(&v[..copy_size]);
		Hash(h)
	}

	/// Converts the hash to a byte vector
	pub fn to_vec(&self) -> Vec<u8> {
		self.0.to_vec()
	}

	/// Convert a hash to hex string format.
	pub fn to_hex(&self) -> String {
		util::to_hex(self.to_vec())
	}

	/// Convert hex string back to hash.
	pub fn from_hex(hex: &str) -> Result<Hash, Error> {
		let bytes = util::from_hex(hex.to_string())
			.map_err(|_| Error::HexError(format!("failed to decode {}", hex)))?;
		Ok(Hash::from_vec(&bytes))
	}

	/// Most significant 64 bits
	pub fn to_u64(&self) -> u64 {
		BigEndian::read_u64(&self.0)
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
		let v = reader.read_fixed_bytes(32)?;
		let mut a = [0; 32];
		a.copy_from_slice(&v[..]);
		Ok(Hash(a))
	}
}

impl Writeable for Hash {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_fixed_bytes(&self.0)
	}
}

impl Add for Hash {
	type Output = Hash;
	fn add(self, other: Hash) -> Hash {
		self.hash_with(other)
	}
}

impl Default for Hash {
	fn default() -> Hash {
		ZERO_HASH
	}
}

/// Serializer that outputs a hash of the serialized object
pub struct HashWriter {
	state: Blake2b,
}

impl HashWriter {
	/// Consume the `HashWriter`, outputting its current hash into a 32-byte
	/// array
	pub fn finalize(self, output: &mut [u8]) {
		output.copy_from_slice(self.state.finalize().as_bytes());
	}

	/// Consume the `HashWriter`, outputting a `Hash` corresponding to its
	/// current state
	pub fn into_hash(self) -> Hash {
		let mut res = [0; 32];
		res.copy_from_slice(self.state.finalize().as_bytes());
		Hash(res)
	}
}

impl Default for HashWriter {
	fn default() -> HashWriter {
		HashWriter {
			state: Blake2b::new(32),
		}
	}
}

impl ser::Writer for HashWriter {
	fn serialization_mode(&self) -> ser::SerializationMode {
		ser::SerializationMode::Hash
	}

	fn write_fixed_bytes<T: AsFixedBytes>(&mut self, b32: &T) -> Result<(), ser::Error> {
		self.state.update(b32.as_ref());
		Ok(())
	}
}

/// A trait for types that have a canonical hash
pub trait Hashed {
	/// Obtain the hash of the object
	fn hash(&self) -> Hash;
	/// Hash the object together with another writeable object
	fn hash_with<T: Writeable>(&self, other: T) -> Hash;
}

impl<W: ser::Writeable> Hashed for W {
	fn hash(&self) -> Hash {
		let mut hasher = HashWriter::default();
		ser::Writeable::write(self, &mut hasher).unwrap();
		let mut ret = [0; 32];
		hasher.finalize(&mut ret);
		Hash(ret)
	}

	fn hash_with<T: Writeable>(&self, other: T) -> Hash {
		let mut hasher = HashWriter::default();
		ser::Writeable::write(self, &mut hasher).unwrap();
		ser::Writeable::write(&other, &mut hasher).unwrap();
		let mut ret = [0; 32];
		hasher.finalize(&mut ret);
		Hash(ret)
	}
}

impl<T: Writeable> consensus::VerifySortOrder<T> for Vec<T> {
	fn verify_sort_order(&self) -> Result<(), consensus::Error> {
		if self
			.iter()
			.map(|item| item.hash())
			.collect::<Vec<_>>()
			.windows(2)
			.any(|pair| pair[0] > pair[1])
		{
			Err(consensus::Error::SortError)
		} else {
			Ok(())
		}
	}
}
