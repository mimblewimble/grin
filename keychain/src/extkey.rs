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

use std::{error, fmt};
use std::cmp::min;

use byteorder::{ByteOrder, BigEndian};
use blake2::blake2b::blake2b;
use secp::Secp256k1;
use secp::key::SecretKey;
use util;

/// An ExtKey error
#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum Error {
	/// The size of the seed is invalid
	InvalidSeedSize,
	InvalidSliceSize,
	InvalidExtendedKey,
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
			Error::InvalidSeedSize => "wallet: seed isn't of size 128, 256 or 512",
			// TODO change when ser. ext. size is fixed
			Error::InvalidSliceSize => "wallet: serialized extended key must be of size 73",
			Error::InvalidExtendedKey => "wallet: the given serialized extended key is invalid",
		}
	}
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
	fn zero() -> Fingerprint {
		Identifier::from_bytes(&[0; 4]).fingerprint()
	}

	fn from_bytes(bytes: &[u8]) -> Fingerprint {
		let mut fingerprint = [0; 4];
		for i in 0..min(4, bytes.len()) {
			fingerprint[i] = bytes[i];
		}
		Fingerprint(util::to_hex(fingerprint.to_vec()))
	}
}

impl fmt::Display for Fingerprint {
	fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
		f.write_str(&self.0)
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier(String);

impl Identifier {
	fn from_bytes(bytes: &[u8]) -> Identifier {
		let mut identifier = [0; 20];
		for i in 0..min(20, bytes.len()) {
			identifier[i] = bytes[i];
		}
		Identifier(util::to_hex(identifier.to_vec()))
	}

	pub fn to_hex(&self) -> String {
		self.0.clone()
	}

	pub fn fingerprint(&self) -> Fingerprint {
		let hex = &self.0[0..8];
		Fingerprint(String::from(hex))
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
	/// Parent key's fingerprint
	pub fingerprint: Fingerprint,
	/// Code of the derivation chain
	pub chaincode: [u8; 32],
	/// Actual private key
	pub key: SecretKey,
}

impl ExtendedKey {
	/// Creates a new extended key from a serialized one
	pub fn from_slice(secp: &Secp256k1, slice: &[u8]) -> Result<ExtendedKey, Error> {
		// TODO change when ser. ext. size is fixed
		if slice.len() != 73 {
			return Err(Error::InvalidSliceSize);
		}
		let depth: u8 = slice[0];
		let fingerprint = Fingerprint::from_bytes(&slice[1..5]);
		let n_child = BigEndian::read_u32(&slice[5..9]);
		let mut chaincode: [u8; 32] = [0; 32];
		(&mut chaincode).copy_from_slice(&slice[9..41]);
		let secret_key = match SecretKey::from_slice(secp, &slice[41..73]) {
			Ok(key) => key,
			Err(_) => return Err(Error::InvalidExtendedKey),
		};

		Ok(ExtendedKey {
			depth: depth,
			fingerprint: fingerprint,
			n_child: n_child,
			chaincode: chaincode,
			key: secret_key,
		})
	}

	/// Creates a new extended master key from a seed
	pub fn from_seed(secp: &Secp256k1, seed: &[u8]) -> Result<ExtendedKey, Error> {
		match seed.len() {
			16 | 32 | 64 => (),
			_ => return Err(Error::InvalidSeedSize),
		}

		let derived = blake2b(64, b"Mimble seed", seed);

		let mut chaincode: [u8; 32] = [0; 32];
		(&mut chaincode).copy_from_slice(&derived.as_bytes()[32..]);
		// TODO Error handling
		let secret_key = SecretKey::from_slice(&secp, &derived.as_bytes()[0..32])
			.expect("Error generating from seed");

		let mut ext_key = ExtendedKey {
			depth: 0,
			fingerprint: Fingerprint::zero(),
			n_child: 0,
			chaincode: chaincode,
			key: secret_key,
		};

		ext_key.fingerprint = ext_key.identifier().fingerprint();

		Ok(ext_key)
	}

	/// Return the identifier of the key
	/// which is the blake2b hash (20 bit digest)
	pub fn identifier(&self) -> Identifier {
		let identifier = blake2b(20, &[], &self.key[..]);
		Identifier::from_bytes(&identifier.as_bytes())
	}

	/// Derive an extended key from an extended key
	pub fn derive(&self, secp: &Secp256k1, n: u32) -> Result<ExtendedKey, Error> {
		let mut n_bytes: [u8; 4] = [0; 4];
		BigEndian::write_u32(&mut n_bytes, n);
		let mut seed = self.key[..].to_vec();
		seed.extend_from_slice(&n_bytes);

		let derived = blake2b(64, &self.chaincode[..], &seed[..]);

		let mut secret_key = SecretKey::from_slice(&secp, &derived.as_bytes()[0..32])
			.expect("Error deriving key");
		secret_key.add_assign(secp, &self.key).expect(
			"Error deriving key",
		);
		// TODO check if key != 0 ?

		let mut chain_code: [u8; 32] = [0; 32];
		(&mut chain_code).clone_from_slice(&derived.as_bytes()[32..]);

		Ok(ExtendedKey {
			depth: self.depth + 1,
			fingerprint: self.identifier().fingerprint(),
			n_child: n,
			chaincode: chain_code,
			key: secret_key,
		})
	}
}

#[cfg(test)]
mod test {
	use secp::Secp256k1;
	use secp::key::SecretKey;
	use super::{ExtendedKey, Fingerprint, Identifier};
	use util;

	fn from_hex(hex_str: &str) -> Vec<u8> {
		util::from_hex(hex_str.to_string()).unwrap()
	}

	#[test]
	fn extkey_from_seed() {
		// TODO More test vectors
		let s = Secp256k1::new();
		let seed = from_hex("000102030405060708090a0b0c0d0e0f");
		let extk = ExtendedKey::from_seed(&s, &seed.as_slice()).unwrap();
		let sec = from_hex(
			"c3f5ae520f474b390a637de4669c84d0ed9bbc21742577fac930834d3c3083dd",
		);
		let secret_key = SecretKey::from_slice(&s, sec.as_slice()).unwrap();
		let chaincode = from_hex(
			"e7298e68452b0c6d54837670896e1aee76b118075150d90d4ee416ece106ae72",
		);
		let identifier = from_hex("942b6c0bd43bdcb24f3edfe7fadbc77054ecc4f2");
		let fingerprint = from_hex("942b6c0b");
		let depth = 0;
		let n_child = 0;
		assert_eq!(extk.key, secret_key);
		assert_eq!(
			extk.identifier(),
			Identifier::from_bytes(identifier.as_slice())
		);
		assert_eq!(
			extk.fingerprint,
			Fingerprint::from_bytes(fingerprint.as_slice())
		);
		assert_eq!(
			extk.identifier().fingerprint(),
			Fingerprint::from_bytes(fingerprint.as_slice())
		);
		assert_eq!(extk.chaincode, chaincode.as_slice());
		assert_eq!(extk.depth, depth);
		assert_eq!(extk.n_child, n_child);
	}

	#[test]
	fn extkey_derivation() {
		// TODO More test vectors
		let s = Secp256k1::new();
		let seed = from_hex("000102030405060708090a0b0c0d0e0f");
		let extk = ExtendedKey::from_seed(&s, &seed.as_slice()).unwrap();
		let derived = extk.derive(&s, 0).unwrap();
		let sec = from_hex(
			"d75f70beb2bd3b56f9b064087934bdedee98e4b5aae6280c58b4eff38847888f",
		);
		let secret_key = SecretKey::from_slice(&s, sec.as_slice()).unwrap();
		let chaincode = from_hex(
			"243cb881e1549e714db31d23af45540b13ad07941f64a786bbf3313b4de1df52",
		);
		let fingerprint = from_hex("942b6c0b");
		let identifier = from_hex("8b011f14345f3f0071e85f6eec116de1e575ea10");
		let identifier_fingerprint = from_hex("8b011f14");
		let depth = 1;
		let n_child = 0;
		assert_eq!(derived.key, secret_key);
		assert_eq!(
			derived.identifier(),
			Identifier::from_bytes(identifier.as_slice())
		);
		assert_eq!(
			derived.fingerprint,
			Fingerprint::from_bytes(fingerprint.as_slice())
		);
		assert_eq!(
			derived.identifier().fingerprint(),
			Fingerprint::from_bytes(identifier_fingerprint.as_slice())
		);
		assert_eq!(derived.chaincode, chaincode.as_slice());
		assert_eq!(derived.depth, depth);
		assert_eq!(derived.n_child, n_child);
	}
}
