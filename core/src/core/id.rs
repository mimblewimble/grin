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

//! short ids for compact blocks

use std::cmp::min;

use byteorder::{LittleEndian, ByteOrder};
use siphasher::sip::SipHasher24;

use core::hash::{Hash, Hashed};
use ser;
use ser::{Reader, Readable, Writer, Writeable};
use util;


/// The size of a short id used to identify inputs|outputs|kernels (6 bytes)
pub const SHORT_ID_SIZE: usize = 6;

/// A trait for types that have a short_id (inputs/outputs/kernels)
pub trait ShortIdentifiable {
	/// The short_id of a kernel uses a hash built from the block_header *and* a
	/// connection specific nonce to minimize the effect of collisions.
	fn short_id(&self, hash: &Hash) -> ShortId;
}

impl<H: Hashed> ShortIdentifiable for H {
	/// Generate a short_id via the following -
	///
	///   * extract k0/k1 from block_hash (first two u64 values)
	///   * initialize a siphasher24 with k0/k1
	///   * self.hash() passing in the siphasher24 instance
	///   * drop the 2 most significant bytes (to return a 6 byte short_id)
	///
	fn short_id(&self, hash: &Hash) -> ShortId {
		// we "use" core::hash::Hash in the outer namespace
		// so doing this here in the fn to minimize collateral damage/confusion
		use std::hash::Hasher;

		// extract k0/k1 from the block_hash
		let k0 = LittleEndian::read_u64(&hash.0[0..8]);
		let k1 = LittleEndian::read_u64(&hash.0[8..16]);

		// initialize a siphasher24 with k0/k1
		let mut sip_hasher = SipHasher24::new_with_keys(k0, k1);

		// hash our id (self.hash()) using the siphasher24 instance
		sip_hasher.write(&self.hash().to_vec()[..]);
		let res = sip_hasher.finish();

		// construct a short_id from the resulting bytes (dropping the 2 most significant bytes)
		let mut buf = [0; 8];
		LittleEndian::write_u64(&mut buf, res);
		ShortId::from_bytes(&buf[0..6])
	}
}

/// Short id for identifying inputs/outputs/kernels
#[derive(PartialEq, Clone, PartialOrd, Ord, Eq, Serialize, Deserialize)]
pub struct ShortId([u8; 6]);

impl ::std::fmt::Debug for ShortId {
	fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
		try!(write!(f, "{}(", stringify!(ShortId)));
		try!(write!(f, "{}", self.to_hex()));
		write!(f, ")")
	}
}

impl Readable for ShortId {
	fn read(reader: &mut Reader) -> Result<ShortId, ser::Error> {
		let v = try!(reader.read_fixed_bytes(SHORT_ID_SIZE));
		let mut a = [0; SHORT_ID_SIZE];
		for i in 0..a.len() {
			a[i] = v[i];
		}
		Ok(ShortId(a))
	}
}

impl Writeable for ShortId {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_fixed_bytes(&self.0)
	}
}

impl ShortId {
	/// Build a new short_id from a byte slice
	pub fn from_bytes(bytes: &[u8]) -> ShortId {
		let mut hash = [0; SHORT_ID_SIZE];
		for i in 0..min(SHORT_ID_SIZE, bytes.len()) {
			hash[i] = bytes[i];
		}
		ShortId(hash)
	}

	/// Hex string representation of a short_id
	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	/// Reconstructs a switch commit hash from a hex string.
	pub fn from_hex(hex: &str) -> Result<ShortId, ser::Error> {
		let bytes = util::from_hex(hex.to_string())
			.map_err(|_| ser::Error::HexError(format!("short_id from_hex error")))?;
		Ok(ShortId::from_bytes(&bytes))
	}

	/// The zero short_id, convenient for generating a short_id for testing.
	pub fn zero() -> ShortId {
		ShortId::from_bytes(&[0])
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use ser::{Writeable, Writer};


	#[test]
	fn test_short_id() {
		// minimal struct for testing
		// make it implement Writeable, therefore Hashable, therefore ShortIdentifiable
		struct Foo(u64);
		impl Writeable for Foo {
			fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
				writer.write_u64(self.0)?;
				Ok(())
			}
		}

		let foo = Foo(0);
		let expected_hash = Hash::from_hex(
			"81e47a19e6b29b0a65b9591762ce5143ed30d0261e5d24a3201752506b20f15c",
		).unwrap();
		assert_eq!(foo.hash(), expected_hash);

		let other_hash = Hash::zero();
		println!("{:?}", foo.short_id(&other_hash));
		assert_eq!(foo.short_id(&other_hash), ShortId::from_hex("e973960ba690").unwrap());

		let foo = Foo(5);
		let expected_hash = Hash::from_hex(
			"3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673",
		).unwrap();
		assert_eq!(foo.hash(), expected_hash);

		let other_hash = Hash::zero();
		println!("{:?}", foo.short_id(&other_hash));
		assert_eq!(foo.short_id(&other_hash), ShortId::from_hex("f0c06e838e59").unwrap());

		let foo = Foo(5);
		let expected_hash = Hash::from_hex(
			"3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673",
		).unwrap();
		assert_eq!(foo.hash(), expected_hash);

		let other_hash = Hash::from_hex(
			"81e47a19e6b29b0a65b9591762ce5143ed30d0261e5d24a3201752506b20f15c",
		).unwrap();
		println!("{:?}", foo.short_id(&other_hash));
		assert_eq!(foo.short_id(&other_hash), ShortId::from_hex("95bf0ca12d5b").unwrap());
	}
}
