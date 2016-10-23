// Copyright 2016 The Developers
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

use std::fmt;

use tiny_keccak::Keccak;

/// A hash to uniquely (or close enough) identify one of the main blockchain
/// constructs. Used pervasively for blocks, transactions and ouputs.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
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
	/// Creates a new hash from a vector
	pub fn from_vec(v: Vec<u8>) -> Hash {
		let mut a = [0; 32];
		for i in 0..a.len() {
			a[i] = v[i];
		}
		Hash(a)
	}
	/// Converts the hash to a byte vector
	pub fn to_vec(&self) -> Vec<u8> {
		self.0.to_vec()
	}
	/// Converts the hash to a byte slice
	pub fn to_slice(&self) -> &[u8] {
		&self.0
	}
}

pub const ZERO_HASH: Hash = Hash([0; 32]);

/// A trait for types that get their hash (double SHA256) from their byte
/// serialzation.
pub trait Hashed {
	fn hash(&self) -> Hash {
		let data = self.bytes();
		Hash(sha3(data))
	}

	fn bytes(&self) -> Vec<u8>;
}

fn sha3(data: Vec<u8>) -> [u8; 32] {
	let mut sha3 = Keccak::new_sha3_256();
	let mut buf = [0; 32];
	sha3.update(&data);
	sha3.finalize(&mut buf);
	buf
}

impl Hashed for [u8] {
	fn bytes(&self) -> Vec<u8> {
		self.to_owned()
	}
}

