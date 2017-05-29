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

/// Key derivation scheme used by Grin to build chains of private keys
/// in its wallet logic. Largely inspired by bitcoin's BIP32.

use std::{error, fmt};

use byteorder::{ByteOrder, BigEndian};
use crypto::mac::Mac;
use crypto::hmac::Hmac;
use crypto::sha2::Sha256;
use crypto::sha2::Sha512;
use crypto::ripemd160::Ripemd160;
use crypto::digest::Digest;
use secp::Secp256k1;
use secp::key::SecretKey;

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
	pub fingerprint: [u8; 4],
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
		let mut fingerprint: [u8; 4] = [0; 4];
		(&mut fingerprint).copy_from_slice(&slice[1..5]);
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
		let mut hmac = Hmac::new(Sha512::new(), b"Mimble seed");
		match seed.len() {
			16 | 32 | 64 => hmac.input(&seed),
			_ => return Err(Error::InvalidSeedSize),
		}

		let mut derived: [u8; 64] = [0; 64];
		hmac.raw_result(&mut derived);

		let mut chaincode: [u8; 32] = [0; 32];
		(&mut chaincode).copy_from_slice(&derived[32..]);
		// TODO Error handling
		let secret_key = SecretKey::from_slice(&secp, &derived[0..32])
			.expect("Error generating from seed");

		let mut ext_key = ExtendedKey {
			depth: 0,
			fingerprint: [0; 4],
			n_child: 0,
			chaincode: chaincode,
			key: secret_key,
		};

		let mut fingerprint: [u8; 4] = [0; 4];
		let identifier = ext_key.identifier();
		(&mut fingerprint).clone_from_slice(&identifier[0..4]);
		ext_key.fingerprint = fingerprint;

		Ok(ext_key)
	}

	/// Return the identifier of the key, which is the
	/// Hash160 of the private key
	pub fn identifier(&self) -> [u8; 20] {
		let mut sha = Sha256::new();
		sha.input(&self.key[..]);

		let mut shres = [0; 32];
		sha.result(&mut shres);

		let mut ripe = Ripemd160::new();
		ripe.input(&shres[..]);

		let mut identifier = [0; 20];
		ripe.result(&mut identifier);
		return identifier;
	}

	/// Derive an extended key from an extended key
	pub fn derive(&self, secp: &Secp256k1, n: u32) -> Result<ExtendedKey, Error> {
		let mut hmac = Hmac::new(Sha512::new(), &self.chaincode[..]);
		let mut n_bytes: [u8; 4] = [0; 4];
		BigEndian::write_u32(&mut n_bytes, n);

		hmac.input(&self.key[..]);
		hmac.input(&n_bytes[..]);

		let mut derived = [0; 64];
		hmac.raw_result(&mut derived);

		let mut secret_key = SecretKey::from_slice(&secp, &derived[0..32])
			.expect("Error deriving key");
		secret_key.add_assign(secp, &self.key)
			.expect("Error deriving key");
		// TODO check if key != 0 ?

		let mut chain_code: [u8; 32] = [0; 32];
		(&mut chain_code).clone_from_slice(&derived[32..]);

		let mut fingerprint: [u8; 4] = [0; 4];
		let parent_identifier = self.identifier();
		(&mut fingerprint).clone_from_slice(&parent_identifier[0..4]);

		Ok(ExtendedKey {
			depth: self.depth + 1,
			fingerprint: fingerprint,
			n_child: n,
			chaincode: chain_code,
			key: secret_key,
		})
	}
}

#[cfg(test)]
mod test {
	extern crate rustc_serialize as serialize;

	use secp::Secp256k1;
	use secp::key::SecretKey;
	use super::ExtendedKey;
	use self::serialize::hex::FromHex;

	#[test]
	fn extkey_from_seed() {
		// TODO More test vectors
		let s = Secp256k1::new();
		let seed = "000102030405060708090a0b0c0d0e0f".from_hex().unwrap();
		let extk = ExtendedKey::from_seed(&s, &seed.as_slice()).unwrap();
		let sec =
			"04a7d66a82221501e67f2665332180bd1192c5e58a2cd26613827deb8ba14e75".from_hex().unwrap();
		let secret_key = SecretKey::from_slice(&s, sec.as_slice()).unwrap();
		let chaincode =
			"b7c6740dea1920ec629b3593678f6d8dc40fe6ec1ed824fcde37f476cd6c048c".from_hex().unwrap();
		let fingerprint = "00000000".from_hex().unwrap();
		let depth = 0;
		let n_child = 0;
		assert_eq!(extk.key, secret_key);
		assert_eq!(extk.fingerprint, fingerprint.as_slice());
		assert_eq!(extk.chaincode, chaincode.as_slice());
		assert_eq!(extk.depth, depth);
		assert_eq!(extk.n_child, n_child);
	}

	#[test]
	fn extkey_derivation() {
		// TODO More test verctors
		let s = Secp256k1::new();
		let seed = "000102030405060708090a0b0c0d0e0f".from_hex().unwrap();
		let extk = ExtendedKey::from_seed(&s, &seed.as_slice()).unwrap();
		let derived = extk.derive(&s, 0).unwrap();
		let sec =
			"908bf3264b8f5f5a5be57d3b0afa36eb5dbcc464ff4da2cf71183e8ec755184b".from_hex().unwrap();
		let secret_key = SecretKey::from_slice(&s, sec.as_slice()).unwrap();
		let chaincode =
			"e90c4559501fb956fa8ddcd6d08499691678cfd6d69e41efb9ee8e87f327e30a".from_hex().unwrap();
		let fingerprint = "8963be69".from_hex().unwrap();
		let depth = 1;
		let n_child = 0;
		assert_eq!(derived.key, secret_key);
		assert_eq!(derived.fingerprint, fingerprint.as_slice());
		assert_eq!(derived.chaincode, chaincode.as_slice());
		assert_eq!(derived.depth, depth);
		assert_eq!(derived.n_child, n_child);
	}
}
