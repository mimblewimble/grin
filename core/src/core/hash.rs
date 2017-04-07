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

use byteorder::{ByteOrder, BigEndian};
use std::fmt;
use tiny_keccak::Keccak;
use std::convert::AsRef;

use ser::{self, Reader, Readable, Writer, Writeable, Error, AsFixedBytes};

pub const ZERO_HASH: Hash = Hash([0; 32]);

/// A hash to uniquely (or close enough) identify one of the main blockchain
/// constructs. Used pervasively for blocks, transactions and ouputs.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl fmt::Display for Hash {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in self.0[..].iter().cloned() {
			try!(write!(f, "{:02x}", i));
		}
		Ok(())
	}
}

impl Hash {
	/// Converts the hash to a byte vector
	pub fn to_vec(&self) -> Vec<u8> {
		self.0.to_vec()
	}
	/// Converts the hash to a byte slice
	pub fn to_slice(&self) -> &[u8] {
		&self.0
	}
}

impl AsRef<[u8]> for Hash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Readable<Hash> for Hash {
	fn read(reader: &mut Reader) -> Result<Hash, ser::Error> {
		let v = try!(reader.read_fixed_bytes(32));
		let mut a = [0; 32];
		for i in 0..a.len() {
			a[i] = v[i];
		}
		Ok(Hash(a))
	}
}

impl Writeable for Hash {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_fixed_bytes(&self.0)
	}
}

/// Serializer that outputs a hash of the serialized object
pub struct HashWriter {
	state: Keccak,
}

impl HashWriter {
	pub fn finalize(self, output: &mut [u8]) {
		self.state.finalize(output);
	}
}

impl Default for HashWriter {
	fn default() -> HashWriter {
		HashWriter { state: Keccak::new_sha3_256() }
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
	fn hash(&self) -> Hash;
}

impl<W: ser::Writeable> Hashed for W {
	fn hash(&self) -> Hash {
		let mut hasher = HashWriter::default();
		ser::Writeable::write(self, &mut hasher).unwrap();
		let mut ret = [0; 32];
		hasher.finalize(&mut ret);
		Hash(ret)
	}
}

// Convenience for when we need to hash of an empty array.
impl Hashed for [u8; 0] {
	fn hash(&self) -> Hash {
		let hasher = HashWriter::default();
		let mut ret = [0; 32];
		hasher.finalize(&mut ret);
		Hash(ret)
	}
}
