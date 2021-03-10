// Copyright 2021 The Grin Developers
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

use crate::core::hash::{DefaultHashable, Hash, Hashed};
use crate::ser::{self, Readable, Reader, Writeable, Writer};
use byteorder::{ByteOrder, LittleEndian};
use siphasher::sip::SipHasher24;
use std::cmp::{min, Ordering};
use util::ToHex;

/// The size of a short id used to identify inputs|outputs|kernels (6 bytes)
pub const SHORT_ID_SIZE: usize = 6;

/// A trait for types that have a short_id (inputs/outputs/kernels)
pub trait ShortIdentifiable {
	/// The short_id of a kernel uses a hash built from the block_header *and* a
	/// connection specific nonce to minimize the effect of collisions.
	fn short_id(&self, hash: &Hash, nonce: u64) -> ShortId;
}

impl<H: Hashed> ShortIdentifiable for H {
	/// Generate a short_id via the following -
	///
	/// * extract k0/k1 from block_hash hashed with the nonce (first two u64
	/// values)   * initialize a siphasher24 with k0/k1
	///   * self.hash() passing in the siphasher24 instance
	///   * drop the 2 most significant bytes (to return a 6 byte short_id)
	///
	fn short_id(&self, hash: &Hash, nonce: u64) -> ShortId {
		// take the block hash and the nonce and hash them together
		let hash_with_nonce = (hash, nonce).hash();

		// we "use" core::hash::Hash in the outer namespace
		// so doing this here in the fn to minimize collateral damage/confusion
		use std::hash::Hasher;

		// extract k0/k1 from the block_hash
		let k0 = LittleEndian::read_u64(&hash_with_nonce.as_bytes()[0..8]);
		let k1 = LittleEndian::read_u64(&hash_with_nonce.as_bytes()[8..16]);

		// initialize a siphasher24 with k0/k1
		let mut sip_hasher = SipHasher24::new_with_keys(k0, k1);

		// hash our id (self.hash()) using the siphasher24 instance
		sip_hasher.write(&self.hash().to_vec()[..]);
		let res = sip_hasher.finish();

		// construct a short_id from the resulting bytes (dropping the 2 most
		// significant bytes)
		let mut buf = [0; 8];
		LittleEndian::write_u64(&mut buf, res);
		ShortId::from_bytes(&buf[0..6])
	}
}

/// Short id for identifying inputs/outputs/kernels
#[derive(Clone, Serialize, Deserialize)]
pub struct ShortId([u8; 6]);

impl DefaultHashable for ShortId {}
// We want to sort short_ids in a canonical and consistent manner so we can
// verify sort order in the same way we do for full inputs|outputs|kernels
// themselves.
hashable_ord!(ShortId);

impl ::std::fmt::Debug for ShortId {
	fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
		write!(f, "{}(", stringify!(ShortId))?;
		write!(f, "{}", self.to_hex())?;
		write!(f, ")")
	}
}

impl AsRef<[u8]> for ShortId {
	fn as_ref(&self) -> &[u8] {
		self.0.as_ref()
	}
}

impl Readable for ShortId {
	fn read<R: Reader>(reader: &mut R) -> Result<ShortId, ser::Error> {
		let v = reader.read_fixed_bytes(SHORT_ID_SIZE)?;
		let mut a = [0; SHORT_ID_SIZE];
		a.copy_from_slice(&v[..]);
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
		let copy_size = min(SHORT_ID_SIZE, bytes.len());
		hash[..copy_size].copy_from_slice(&bytes[..copy_size]);
		ShortId(hash)
	}

	/// Reconstructs a switch commit hash from a hex string.
	pub fn from_hex(hex: &str) -> Result<ShortId, ser::Error> {
		let bytes = util::from_hex(hex)
			.map_err(|_| ser::Error::HexError("short_id from_hex error".to_string()))?;
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
	use crate::ser::{Writeable, Writer};

	#[test]
	fn short_id_ord() {
		let id_1 = ShortId::from_bytes(&[1, 1, 1, 1]);
		let id_2 = ShortId::from_bytes(&[2, 2, 2, 2]);
		let id_3 = ShortId::from_bytes(&[3, 3, 3, 3]);

		let mut ids = vec![id_1.clone(), id_2.clone(), id_3.clone()];
		println!("{:?}", ids);

		let mut hashes = ids.iter().map(|x| x.hash()).collect::<Vec<_>>();
		println!("{:?}", hashes);

		// NOTE: after sorting hash(3) comes before hash(2)
		hashes.sort();
		println!("{:?}", hashes);
		assert_eq!(hashes, [id_1.hash(), id_3.hash(), id_2.hash()]);

		// NOTE: this also applies to sorting the ids (we sort based on hashes)
		ids.sort();
		println!("{:?}", ids);
		assert_eq!(ids, [id_1, id_3, id_2]);
	}

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

		impl DefaultHashable for Foo {}

		let foo = Foo(0);

		let expected_hash =
			Hash::from_hex("81e47a19e6b29b0a65b9591762ce5143ed30d0261e5d24a3201752506b20f15c")
				.unwrap();
		assert_eq!(foo.hash(), expected_hash);

		let other_hash = Hash::default();
		assert_eq!(
			foo.short_id(&other_hash, foo.0),
			ShortId::from_hex("4cc808b62476").unwrap()
		);

		let foo = Foo(5);
		let expected_hash =
			Hash::from_hex("3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673")
				.unwrap();
		assert_eq!(foo.hash(), expected_hash);

		let other_hash = Hash::default();
		assert_eq!(
			foo.short_id(&other_hash, foo.0),
			ShortId::from_hex("02955a094534").unwrap()
		);

		let foo = Foo(5);
		let expected_hash =
			Hash::from_hex("3a42e66e46dd7633b57d1f921780a1ac715e6b93c19ee52ab714178eb3a9f673")
				.unwrap();
		assert_eq!(foo.hash(), expected_hash);

		let other_hash =
			Hash::from_hex("81e47a19e6b29b0a65b9591762ce5143ed30d0261e5d24a3201752506b20f15c")
				.unwrap();
		assert_eq!(
			foo.short_id(&other_hash, foo.0),
			ShortId::from_hex("3e9cde72a687").unwrap()
		);
	}
}
