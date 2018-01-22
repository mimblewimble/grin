// Copyright 2017 The Grin Developers
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

use std::{error, fmt, num};
use std::cmp::min;

use serde::{de, ser};

use byteorder::{BigEndian, ByteOrder};
use blake2::blake2b::blake2b;
use util::secp;
use util::secp::Secp256k1;
use util::secp::key::{PublicKey, SecretKey};
use util;

// Size of an identifier in bytes
pub const IDENTIFIER_SIZE: usize = 10;

/// An ExtKey error
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Error {
	/// The size of the seed is invalid
	InvalidSeedSize,
	InvalidSliceSize,
	InvalidExtendedKey,
	Secp(secp::Error),
	ParseIntError(num::ParseIntError),
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<num::ParseIntError> for Error {
	fn from(e: num::ParseIntError) -> Error {
		Error::ParseIntError(e)
	}
}

// Passthrough Debug to Display, since errors should be user-visible
impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		f.write_str(error::Error::description(self))
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&error::Error> {
		None
	}

	fn description(&self) -> &str {
		match *self {
			Error::InvalidSeedSize => "keychain: seed isn't of size 128, 256 or 512",
			// TODO change when ser. ext. size is fixed
			Error::InvalidSliceSize => "keychain: serialized extended key must be of size 73",
			Error::InvalidExtendedKey => "keychain: the given serialized extended key is invalid",
			Error::Secp(_) => "keychain: secp error",
			Error::ParseIntError(_) => "keychain: error parsing int",
		}
	}
}

#[derive(Clone, PartialEq, Eq, Ord, Hash, PartialOrd)]
pub struct Identifier([u8; IDENTIFIER_SIZE]);

impl ser::Serialize for Identifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: ser::Serializer,
	{
		serializer.serialize_str(&self.to_hex())
	}
}

impl<'de> de::Deserialize<'de> for Identifier {
	fn deserialize<D>(deserializer: D) -> Result<Identifier, D::Error>
	where
		D: de::Deserializer<'de>,
	{
		deserializer.deserialize_str(IdentifierVisitor)
	}
}

struct IdentifierVisitor;

impl<'de> de::Visitor<'de> for IdentifierVisitor {
	type Value = Identifier;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("an identifier")
	}

	fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
	where
		E: de::Error,
	{
		let identifier = Identifier::from_hex(s).unwrap();
		Ok(identifier)
	}
}

impl Identifier {
	pub fn zero() -> Identifier {
		Identifier::from_bytes(&[0; IDENTIFIER_SIZE])
	}

	pub fn from_bytes(bytes: &[u8]) -> Identifier {
		let mut identifier = [0; IDENTIFIER_SIZE];
		for i in 0..min(IDENTIFIER_SIZE, bytes.len()) {
			identifier[i] = bytes[i];
		}
		Identifier(identifier)
	}

	pub fn from_key_id(secp: &Secp256k1, pubkey: &PublicKey) -> Identifier {
		let bytes = pubkey.serialize_vec(secp, true);
		let identifier = blake2b(IDENTIFIER_SIZE, &[], &bytes[..]);
		Identifier::from_bytes(&identifier.as_bytes())
	}

	fn from_hex(hex: &str) -> Result<Identifier, Error> {
		let bytes = util::from_hex(hex.to_string()).unwrap();
		Ok(Identifier::from_bytes(&bytes))
	}

	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}
}

impl AsRef<[u8]> for Identifier {
	fn as_ref(&self) -> &[u8] {
		&self.0.as_ref()
	}
}

impl ::std::fmt::Debug for Identifier {
	fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
		try!(write!(f, "{}(", stringify!(Identifier)));
		try!(write!(f, "{}", self.to_hex()));
		write!(f, ")")
	}
}

impl fmt::Display for Identifier {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.to_hex())
	}
}

/// An ExtendedKey is a secret key which can be used to derive new
/// secret keys to blind the commitment of a transaction output.
/// To be usable, a secret key should have an amount assigned to it,
/// but when the key is derived, the amount is not known and must be
/// given.
#[derive(Debug, Clone)]
pub struct ExtendedKey {
	/// Depth of the extended key
	pub depth: u8,
	/// Child number of the key
	pub n_child: u32,
	/// Root key identifier
	pub root_key_id: Identifier,
	/// Actual private key
	pub key: SecretKey,
	/// Code of the derivation chain
	pub chaincode: [u8; 32],
	/// The bytes of the key used for generating the associated switch_commit_hash
	pub switch_key: [u8; 32],
	/// Code of the derivation chain for the switch_commit_hash key
	pub switch_chaincode: [u8; 32],
}

impl ExtendedKey {
	/// Creates a new extended master key from a seed
	pub fn from_seed(secp: &Secp256k1, seed: &[u8]) -> Result<ExtendedKey, Error> {
		match seed.len() {
			16 | 32 | 64 => (),
			_ => return Err(Error::InvalidSeedSize),
		}

		let derived = blake2b(64, b"Grin/MW Seed", seed);
		let slice = derived.as_bytes();

		// TODO Error handling
		let key = SecretKey::from_slice(&secp, &slice[0..32])
			.expect("Error generating from seed");

		let mut chaincode: [u8; 32] = Default::default();
		(&mut chaincode).copy_from_slice(&slice[32..64]);

		// Now derive the switch_key and switch_chaincode in a similar fashion
		// but using a different key to ensure there is nothing linking
		// the secret key and the switch commit hash key for any extended key
		// we subsequently derive
		let switch_derived = blake2b(64, b"Grin/MW Switch Seed", seed);
		let switch_slice = switch_derived.as_bytes();

		let mut switch_key: [u8; 32] = Default::default();
		(&mut switch_key).copy_from_slice(&switch_slice[0..32]);
		let mut switch_chaincode: [u8; 32] = Default::default();
		(&mut switch_chaincode).copy_from_slice(&switch_slice[32..64]);

		let mut ext_key = ExtendedKey {
			depth: 0,
			root_key_id: Identifier::zero(),
			n_child: 0,
			key,
			chaincode,
			switch_key,
			switch_chaincode,
		};

		ext_key.root_key_id = ext_key.identifier(secp)?;

		Ok(ext_key)
	}

	/// Return the identifier of the key
	/// which is the blake2b (10 byte) digest of the PublicKey
	// corresponding to the underlying SecretKey
	pub fn identifier(&self, secp: &Secp256k1) -> Result<Identifier, Error> {
		let key_id = PublicKey::from_secret_key(secp, &self.key)?;
		Ok(Identifier::from_key_id(secp, &key_id))
	}

	/// Derive an extended key from an extended key
	pub fn derive(&self, secp: &Secp256k1, n: u32) -> Result<ExtendedKey, Error> {
		let mut n_bytes: [u8; 4] = [0; 4];
		BigEndian::write_u32(&mut n_bytes, n);

		let mut seed = self.key[..].to_vec();
		seed.extend_from_slice(&n_bytes);

		let derived = blake2b(64, &self.chaincode[..], &seed[..]);
		let slice = derived.as_bytes();

		let mut key = SecretKey::from_slice(&secp, &slice[0..32])
			.expect("Error deriving key");
		key.add_assign(secp, &self.key)
			.expect("Error deriving key");

		let mut chaincode: [u8; 32] = Default::default();
		(&mut chaincode).copy_from_slice(&slice[32..64]);

		// Now derive the switch_key and switch_chaincode in a similar fashion
		let mut switch_seed = self.switch_key[..].to_vec();
		switch_seed.extend_from_slice(&n_bytes);
		let switch_derived = blake2b(64, &self.switch_chaincode[..], &switch_seed[..]);
		let switch_slice = switch_derived.as_bytes();
		let mut switch_key: [u8; 32] = Default::default();
		(&mut switch_key).copy_from_slice(&switch_slice[0..32]);
		let mut switch_chaincode: [u8; 32] = Default::default();
		(&mut switch_chaincode).copy_from_slice(&switch_slice[32..64]);

		// TODO check if key != 0 ?

		Ok(ExtendedKey {
			depth: self.depth + 1,
			root_key_id: self.identifier(&secp)?,
			n_child: n,
			key,
			chaincode,
			switch_key,
			switch_chaincode,
		})
	}
}

#[cfg(test)]
mod test {
	use serde_json;

	use util::secp::Secp256k1;
	use util::secp::key::SecretKey;
	use super::{ExtendedKey, Identifier};
	use util;

	fn from_hex(hex_str: &str) -> Vec<u8> {
		util::from_hex(hex_str.to_string()).unwrap()
	}

	#[test]
	fn test_identifier_json_ser_deser() {
		let hex = "942b6c0bd43bdcb24f3edfe7fadbc77054ecc4f2";
		let identifier = Identifier::from_hex(hex).unwrap();

		#[derive(Debug, Serialize, Deserialize, PartialEq)]
		struct HasAnIdentifier {
			identifier: Identifier,
		}

		let has_an_identifier = HasAnIdentifier { identifier };

		let json = serde_json::to_string(&has_an_identifier).unwrap();
		assert_eq!(json, "{\"identifier\":\"942b6c0bd43bdcb24f3e\"}");

		let deserialized: HasAnIdentifier = serde_json::from_str(&json).unwrap();
		assert_eq!(deserialized, has_an_identifier);
	}

	#[test]
	fn extkey_from_seed() {
		// TODO More test vectors
		let s = Secp256k1::new();
		let seed = from_hex("000102030405060708090a0b0c0d0e0f");
		let extk = ExtendedKey::from_seed(&s, &seed.as_slice()).unwrap();
		let sec = from_hex("2878a92133b0a7c2fbfb0bd4520ed2e55ea3fa2913200f05c30077d30b193480");
		let secret_key = SecretKey::from_slice(&s, sec.as_slice()).unwrap();
		let chaincode =
			from_hex("3ad40dd836c5ce25dfcbdee5044d92cf6b65bd5475717fa7a56dd4a032cca7c0");
		let identifier = from_hex("6f7c1a053ca54592e783");
		let depth = 0;
		let n_child = 0;
		assert_eq!(extk.key, secret_key);
		assert_eq!(
			extk.identifier(&s).unwrap(),
			Identifier::from_bytes(identifier.as_slice())
		);
		assert_eq!(
			extk.root_key_id,
			Identifier::from_bytes(identifier.as_slice())
		);
		assert_eq!(extk.chaincode, chaincode.as_slice());
		assert_eq!(extk.depth, depth);
		assert_eq!(extk.n_child, n_child);
	}

	#[test]
	fn extkey_derivation() {
		let s = Secp256k1::new();
		let seed = from_hex("000102030405060708090a0b0c0d0e0f");
		let extk = ExtendedKey::from_seed(&s, &seed.as_slice()).unwrap();
		let derived = extk.derive(&s, 0).unwrap();
		let sec = from_hex("2676a3ab2ded7c79cbd0bd26d448698de5da5af8e809080d3cacfa2ee31a9aa7");
		let secret_key = SecretKey::from_slice(&s, sec.as_slice()).unwrap();
		let chaincode =
			from_hex("9bc90b148f4c9478205d6ca72c58bbda2902be1e5082de05d56339a74a5314a3");
		let root_key_id = from_hex("6f7c1a053ca54592e783");
		let identifier = from_hex("5f2ec8ee00e8bca002fa");
		let depth = 1;
		let n_child = 0;
		assert_eq!(derived.key, secret_key);
		assert_eq!(
			derived.identifier(&s).unwrap(),
			Identifier::from_bytes(identifier.as_slice())
		);
		assert_eq!(
			derived.root_key_id,
			Identifier::from_bytes(root_key_id.as_slice())
		);
		assert_eq!(derived.chaincode, chaincode.as_slice());
		assert_eq!(derived.depth, depth);
		assert_eq!(derived.n_child, n_child);
	}
}
