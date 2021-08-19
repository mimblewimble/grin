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

// Rust Bitcoin Library
// Written in 2014 by
//     Andrew Poelstra <apoelstra@wpsoftware.net>
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! Implementation of BIP32 hierarchical deterministic wallets, as defined
//! at https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki
//! Modified from above to integrate into grin and allow for different
//! hashing algorithms if desired

#[cfg(feature = "serde")]
use serde;
use std::default::Default;
use std::io::Cursor;
use std::str::FromStr;
use std::{error, fmt};

use crate::mnemonic;
use crate::util::secp::key::{PublicKey, SecretKey};
use crate::util::secp::{self, ContextFlag, Secp256k1};
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};

use digest::generic_array::GenericArray;
use digest::Digest;
use hmac::{Hmac, Mac, NewMac};
use ripemd160::Ripemd160;
use sha2::{Sha256, Sha512};

use crate::base58;

// Create alias for HMAC-SHA512
type HmacSha512 = Hmac<Sha512>;

/// A chain code
pub struct ChainCode([u8; 32]);
impl_array_newtype!(ChainCode, u8, 32);
impl_array_newtype_show!(ChainCode);
impl_array_newtype_encodable!(ChainCode, u8, 32);

/// A fingerprint
pub struct Fingerprint([u8; 4]);
impl_array_newtype!(Fingerprint, u8, 4);
impl_array_newtype_show!(Fingerprint);
impl_array_newtype_encodable!(Fingerprint, u8, 4);

impl Default for Fingerprint {
	fn default() -> Fingerprint {
		Fingerprint([0, 0, 0, 0])
	}
}

/// Allow different implementations of hash functions used in BIP32 Derivations
/// Grin uses blake2 everywhere but the spec calls for SHA512/Ripemd160, so allow
/// this in future and allow us to unit test against published BIP32 test vectors
/// The function names refer to the place of the hash in the reference BIP32 spec,
/// not what the actual implementation is

pub trait BIP32Hasher {
	fn network_priv(&self) -> [u8; 4];
	fn network_pub(&self) -> [u8; 4];
	fn master_seed() -> [u8; 12];
	fn init_sha512(&mut self, seed: &[u8]);
	fn append_sha512(&mut self, value: &[u8]);
	fn result_sha512(&mut self) -> [u8; 64];
	fn sha_256(&self, input: &[u8]) -> [u8; 32];
	fn ripemd_160(&self, input: &[u8]) -> [u8; 20];
}

/// Implementation of the above that uses the standard BIP32 Hash algorithms
#[derive(Clone, Debug)]
pub struct BIP32GrinHasher {
	is_test: bool,
	hmac_sha512: Hmac<Sha512>,
}

impl BIP32GrinHasher {
	/// New empty hasher
	pub fn new(is_test: bool) -> BIP32GrinHasher {
		BIP32GrinHasher {
			is_test: is_test,
			hmac_sha512: HmacSha512::new(GenericArray::from_slice(&[0u8; 128])),
		}
	}
}

impl BIP32Hasher for BIP32GrinHasher {
	fn network_priv(&self) -> [u8; 4] {
		if self.is_test {
			[0x03, 0x27, 0x3A, 0x10]
		} else {
			[0x03, 0x3C, 0x04, 0xA4]
		}
	}
	fn network_pub(&self) -> [u8; 4] {
		if self.is_test {
			[0x03, 0x27, 0x3E, 0x4B]
		} else {
			[0x03, 0x3C, 0x08, 0xDF]
		}
	}
	fn master_seed() -> [u8; 12] {
		b"IamVoldemort".to_owned()
	}
	fn init_sha512(&mut self, seed: &[u8]) {
		self.hmac_sha512 = HmacSha512::new_from_slice(seed).expect("HMAC can take key of any size");
	}
	fn append_sha512(&mut self, value: &[u8]) {
		self.hmac_sha512.update(value);
	}
	fn result_sha512(&mut self) -> [u8; 64] {
		let mut result = [0; 64];
		result.copy_from_slice(&self.hmac_sha512.to_owned().finalize().into_bytes());
		result
	}
	fn sha_256(&self, input: &[u8]) -> [u8; 32] {
		let mut sha2_res = [0; 32];
		let mut sha2 = Sha256::new();
		sha2.update(input);
		sha2_res.copy_from_slice(sha2.finalize().as_slice());
		sha2_res
	}
	fn ripemd_160(&self, input: &[u8]) -> [u8; 20] {
		let mut ripemd_res = [0; 20];
		let mut ripemd = Ripemd160::new();
		ripemd.update(input);
		ripemd_res.copy_from_slice(ripemd.finalize().as_slice());
		ripemd_res
	}
}

/// Extended private key
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExtendedPrivKey {
	/// The network this key is to be used on
	pub network: [u8; 4],
	/// How many derivations this key is from the master (which is 0)
	pub depth: u8,
	/// Fingerprint of the parent key (0 for master)
	pub parent_fingerprint: Fingerprint,
	/// Child number of the key used to derive from parent (0 for master)
	pub child_number: ChildNumber,
	/// Secret key
	pub secret_key: SecretKey,
	/// Chain code
	pub chain_code: ChainCode,
}

/// Extended public key
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ExtendedPubKey {
	/// The network this key is to be used on
	pub network: [u8; 4],
	/// How many derivations this key is from the master (which is 0)
	pub depth: u8,
	/// Fingerprint of the parent key
	pub parent_fingerprint: Fingerprint,
	/// Child number of the key used to derive from parent (0 for master)
	pub child_number: ChildNumber,
	/// Public key
	pub public_key: PublicKey,
	/// Chain code
	pub chain_code: ChainCode,
}

/// A child number for a derived key
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ChildNumber {
	/// Non-hardened key
	Normal {
		/// Key index, within [0, 2^31 - 1]
		index: u32,
	},
	/// Hardened key
	Hardened {
		/// Key index, within [0, 2^31 - 1]
		index: u32,
	},
}

impl ChildNumber {
	/// Create a [`Normal`] from an index, panics if the index is not within
	/// [0, 2^31 - 1].
	///
	/// [`Normal`]: #variant.Normal
	pub fn from_normal_idx(index: u32) -> Self {
		assert_eq!(
			index & (1 << 31),
			0,
			"ChildNumber indices have to be within [0, 2^31 - 1], is: {}",
			index
		);
		ChildNumber::Normal { index: index }
	}

	/// Create a [`Hardened`] from an index, panics if the index is not within
	/// [0, 2^31 - 1].
	///
	/// [`Hardened`]: #variant.Hardened
	pub fn from_hardened_idx(index: u32) -> Self {
		assert_eq!(
			index & (1 << 31),
			0,
			"ChildNumber indices have to be within [0, 2^31 - 1], is: {}",
			index
		);
		ChildNumber::Hardened { index: index }
	}

	/// Returns `true` if the child number is a [`Normal`] value.
	///
	/// [`Normal`]: #variant.Normal
	pub fn is_normal(self) -> bool {
		!self.is_hardened()
	}

	/// Returns `true` if the child number is a [`Hardened`] value.
	///
	/// [`Hardened`]: #variant.Hardened
	pub fn is_hardened(self) -> bool {
		match self {
			ChildNumber::Hardened { .. } => true,
			ChildNumber::Normal { .. } => false,
		}
	}
}

impl From<u32> for ChildNumber {
	fn from(number: u32) -> Self {
		if number & (1 << 31) != 0 {
			ChildNumber::Hardened {
				index: number ^ (1 << 31),
			}
		} else {
			ChildNumber::Normal { index: number }
		}
	}
}

impl From<ChildNumber> for u32 {
	fn from(cnum: ChildNumber) -> Self {
		match cnum {
			ChildNumber::Normal { index } => index,
			ChildNumber::Hardened { index } => index | (1 << 31),
		}
	}
}

impl fmt::Display for ChildNumber {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			ChildNumber::Hardened { index } => write!(f, "{}'", index),
			ChildNumber::Normal { index } => write!(f, "{}", index),
		}
	}
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ChildNumber {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		u32::deserialize(deserializer).map(ChildNumber::from)
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for ChildNumber {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		u32::from(*self).serialize(serializer)
	}
}

/// A BIP32 error
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Error {
	/// A pk->pk derivation was attempted on a hardened key
	CannotDeriveFromHardenedKey,
	/// A secp256k1 error occured
	Ecdsa(secp::Error),
	/// A child number was provided that was out of range
	InvalidChildNumber(ChildNumber),
	/// Error creating a master seed --- for application use
	RngError(String),
	/// Error converting mnemonic to seed
	MnemonicError(mnemonic::Error),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			Error::CannotDeriveFromHardenedKey => {
				f.write_str("cannot derive hardened key from public key")
			}
			Error::Ecdsa(ref e) => fmt::Display::fmt(e, f),
			Error::InvalidChildNumber(ref n) => write!(f, "child number {} is invalid", n),
			Error::RngError(ref s) => write!(f, "rng error {}", s),
			Error::MnemonicError(ref e) => fmt::Display::fmt(e, f),
		}
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&dyn error::Error> {
		if let Error::Ecdsa(ref e) = *self {
			Some(e)
		} else {
			None
		}
	}
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Ecdsa(e)
	}
}

impl ExtendedPrivKey {
	/// Construct a new master key from a seed value
	pub fn new_master<H>(
		secp: &Secp256k1,
		hasher: &mut H,
		seed: &[u8],
	) -> Result<ExtendedPrivKey, Error>
	where
		H: BIP32Hasher,
	{
		hasher.init_sha512(&H::master_seed());
		hasher.append_sha512(seed);
		let result = hasher.result_sha512();

		Ok(ExtendedPrivKey {
			network: hasher.network_priv(),
			depth: 0,
			parent_fingerprint: Default::default(),
			child_number: ChildNumber::from_normal_idx(0),
			secret_key: SecretKey::from_slice(secp, &result[..32]).map_err(Error::Ecdsa)?,
			chain_code: ChainCode::from(&result[32..]),
		})
	}

	/// Construct a new master key from a mnemonic and a passphrase
	pub fn from_mnemonic(
		secp: &Secp256k1,
		mnemonic: &str,
		passphrase: &str,
		is_test: bool,
	) -> Result<ExtendedPrivKey, Error> {
		let seed = mnemonic::to_seed(mnemonic, passphrase).map_err(Error::MnemonicError)?;
		let mut hasher = BIP32GrinHasher::new(is_test);
		let key = ExtendedPrivKey::new_master(secp, &mut hasher, &seed)?;
		Ok(key)
	}

	/// Attempts to derive an extended private key from a path.
	pub fn derive_priv<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		cnums: &[ChildNumber],
	) -> Result<ExtendedPrivKey, Error>
	where
		H: BIP32Hasher,
	{
		let mut sk: ExtendedPrivKey = self.clone();
		for cnum in cnums {
			sk = sk.ckd_priv(secp, hasher, *cnum)?;
		}
		Ok(sk)
	}

	/// Private->Private child key derivation
	pub fn ckd_priv<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		i: ChildNumber,
	) -> Result<ExtendedPrivKey, Error>
	where
		H: BIP32Hasher,
	{
		hasher.init_sha512(&self.chain_code[..]);
		let mut be_n = [0; 4];
		match i {
			ChildNumber::Normal { .. } => {
				// Non-hardened key: compute public data and use that
				hasher.append_sha512(
					&PublicKey::from_secret_key(secp, &self.secret_key)?.serialize_vec(secp, true)
						[..],
				);
			}
			ChildNumber::Hardened { .. } => {
				// Hardened key: use only secret data to prevent public derivation
				hasher.append_sha512(&[0u8]);
				hasher.append_sha512(&self.secret_key[..]);
			}
		}
		BigEndian::write_u32(&mut be_n, u32::from(i));

		hasher.append_sha512(&be_n);
		let result = hasher.result_sha512();
		let mut sk = SecretKey::from_slice(secp, &result[..32]).map_err(Error::Ecdsa)?;
		sk.add_assign(secp, &self.secret_key)
			.map_err(Error::Ecdsa)?;

		Ok(ExtendedPrivKey {
			network: self.network,
			depth: self.depth + 1,
			parent_fingerprint: self.fingerprint(hasher),
			child_number: i,
			secret_key: sk,
			chain_code: ChainCode::from(&result[32..]),
		})
	}

	/// Returns the HASH160 of the chaincode
	pub fn identifier<H>(&self, hasher: &mut H) -> [u8; 20]
	where
		H: BIP32Hasher,
	{
		let secp = Secp256k1::with_caps(ContextFlag::SignOnly);
		// Compute extended public key
		let pk: ExtendedPubKey = ExtendedPubKey::from_private::<H>(&secp, self, hasher);
		// Do SHA256 of just the ECDSA pubkey
		let sha2_res = hasher.sha_256(&pk.public_key.serialize_vec(&secp, true)[..]);
		// do RIPEMD160
		hasher.ripemd_160(&sha2_res)
	}

	/// Returns the first four bytes of the identifier
	pub fn fingerprint<H>(&self, hasher: &mut H) -> Fingerprint
	where
		H: BIP32Hasher,
	{
		Fingerprint::from(&self.identifier(hasher)[0..4])
	}
}

impl ExtendedPubKey {
	/// Derives a public key from a private key
	pub fn from_private<H>(secp: &Secp256k1, sk: &ExtendedPrivKey, hasher: &mut H) -> ExtendedPubKey
	where
		H: BIP32Hasher,
	{
		ExtendedPubKey {
			network: hasher.network_pub(),
			depth: sk.depth,
			parent_fingerprint: sk.parent_fingerprint,
			child_number: sk.child_number,
			public_key: PublicKey::from_secret_key(secp, &sk.secret_key).unwrap(),
			chain_code: sk.chain_code,
		}
	}

	/// Attempts to derive an extended public key from a path.
	pub fn derive_pub<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		cnums: &[ChildNumber],
	) -> Result<ExtendedPubKey, Error>
	where
		H: BIP32Hasher,
	{
		let mut pk: ExtendedPubKey = *self;
		for cnum in cnums {
			pk = pk.ckd_pub(secp, hasher, *cnum)?
		}
		Ok(pk)
	}

	/// Compute the scalar tweak added to this key to get a child key
	pub fn ckd_pub_tweak<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		i: ChildNumber,
	) -> Result<(SecretKey, ChainCode), Error>
	where
		H: BIP32Hasher,
	{
		match i {
			ChildNumber::Hardened { .. } => Err(Error::CannotDeriveFromHardenedKey),
			ChildNumber::Normal { index: n } => {
				hasher.init_sha512(&self.chain_code[..]);
				hasher.append_sha512(&self.public_key.serialize_vec(secp, true)[..]);
				let mut be_n = [0; 4];
				BigEndian::write_u32(&mut be_n, n);
				hasher.append_sha512(&be_n);

				let result = hasher.result_sha512();

				let secret_key = SecretKey::from_slice(secp, &result[..32])?;
				let chain_code = ChainCode::from(&result[32..]);
				Ok((secret_key, chain_code))
			}
		}
	}

	/// Public->Public child key derivation
	pub fn ckd_pub<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		i: ChildNumber,
	) -> Result<ExtendedPubKey, Error>
	where
		H: BIP32Hasher,
	{
		let (sk, chain_code) = self.ckd_pub_tweak(secp, hasher, i)?;
		let mut pk = self.public_key;
		pk.add_exp_assign(secp, &sk).map_err(Error::Ecdsa)?;

		Ok(ExtendedPubKey {
			network: self.network,
			depth: self.depth + 1,
			parent_fingerprint: self.fingerprint(secp, hasher),
			child_number: i,
			public_key: pk,
			chain_code: chain_code,
		})
	}

	/// Returns the HASH160 of the chaincode
	pub fn identifier<H>(&self, secp: &Secp256k1, hasher: &mut H) -> [u8; 20]
	where
		H: BIP32Hasher,
	{
		// Do SHA256 of just the ECDSA pubkey
		let sha2_res = hasher.sha_256(&self.public_key.serialize_vec(secp, true)[..]);
		// do RIPEMD160
		hasher.ripemd_160(&sha2_res)
	}

	/// Returns the first four bytes of the identifier
	pub fn fingerprint<H>(&self, secp: &Secp256k1, hasher: &mut H) -> Fingerprint
	where
		H: BIP32Hasher,
	{
		Fingerprint::from(&self.identifier(secp, hasher)[0..4])
	}
}

impl fmt::Display for ExtendedPrivKey {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut ret = [0; 78];
		ret[0..4].copy_from_slice(&self.network[0..4]);
		ret[4] = self.depth as u8;
		ret[5..9].copy_from_slice(&self.parent_fingerprint[..]);

		BigEndian::write_u32(&mut ret[9..13], u32::from(self.child_number));

		ret[13..45].copy_from_slice(&self.chain_code[..]);
		ret[45] = 0;
		ret[46..78].copy_from_slice(&self.secret_key[..]);
		fmt.write_str(&base58::check_encode_slice(&ret[..]))
	}
}

impl FromStr for ExtendedPrivKey {
	type Err = base58::Error;

	fn from_str(inp: &str) -> Result<ExtendedPrivKey, base58::Error> {
		let s = Secp256k1::without_caps();
		let data = base58::from_check(inp)?;

		if data.len() != 78 {
			return Err(base58::Error::InvalidLength(data.len()));
		}

		let cn_int: u32 = Cursor::new(&data[9..13]).read_u32::<BigEndian>().unwrap();
		let child_number: ChildNumber = ChildNumber::from(cn_int);

		let mut network = [0; 4];
		network.copy_from_slice(&data[0..4]);

		Ok(ExtendedPrivKey {
			network: network,
			depth: data[4],
			parent_fingerprint: Fingerprint::from(&data[5..9]),
			child_number: child_number,
			chain_code: ChainCode::from(&data[13..45]),
			secret_key: SecretKey::from_slice(&s, &data[46..78])
				.map_err(|e| base58::Error::Other(e.to_string()))?,
		})
	}
}

impl fmt::Display for ExtendedPubKey {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		let secp = Secp256k1::without_caps();
		let mut ret = [0; 78];
		ret[0..4].copy_from_slice(&self.network[0..4]);
		ret[4] = self.depth as u8;
		ret[5..9].copy_from_slice(&self.parent_fingerprint[..]);

		BigEndian::write_u32(&mut ret[9..13], u32::from(self.child_number));

		ret[13..45].copy_from_slice(&self.chain_code[..]);
		ret[45..78].copy_from_slice(&self.public_key.serialize_vec(&secp, true)[..]);
		fmt.write_str(&base58::check_encode_slice(&ret[..]))
	}
}

impl FromStr for ExtendedPubKey {
	type Err = base58::Error;

	fn from_str(inp: &str) -> Result<ExtendedPubKey, base58::Error> {
		let s = Secp256k1::without_caps();
		let data = base58::from_check(inp)?;

		if data.len() != 78 {
			return Err(base58::Error::InvalidLength(data.len()));
		}

		let cn_int: u32 = Cursor::new(&data[9..13]).read_u32::<BigEndian>().unwrap();
		let child_number: ChildNumber = ChildNumber::from(cn_int);

		let mut network = [0; 4];
		network.copy_from_slice(&data[0..4]);

		Ok(ExtendedPubKey {
			network: network,
			depth: data[4],
			parent_fingerprint: Fingerprint::from(&data[5..9]),
			child_number: child_number,
			chain_code: ChainCode::from(&data[13..45]),
			public_key: PublicKey::from_slice(&s, &data[45..78])
				.map_err(|e| base58::Error::Other(e.to_string()))?,
		})
	}
}

#[cfg(test)]
mod tests {

	use std::str::FromStr;
	use std::string::ToString;

	use crate::util::from_hex;
	use crate::util::secp::Secp256k1;

	use super::*;

	use digest::generic_array::GenericArray;
	use digest::Digest;
	use hmac::{Hmac, Mac};
	use ripemd160::Ripemd160;
	use sha2::{Sha256, Sha512};

	/// Implementation of the above that uses the standard BIP32 Hash algorithms
	pub struct BIP32ReferenceHasher {
		hmac_sha512: Hmac<Sha512>,
	}

	impl BIP32ReferenceHasher {
		/// New empty hasher
		pub fn new() -> BIP32ReferenceHasher {
			BIP32ReferenceHasher {
				hmac_sha512: HmacSha512::new(GenericArray::from_slice(&[0u8; 128])),
			}
		}
	}

	impl BIP32Hasher for BIP32ReferenceHasher {
		fn network_priv(&self) -> [u8; 4] {
			// bitcoin network (xprv) (for test vectors)
			[0x04, 0x88, 0xAD, 0xE4]
		}
		fn network_pub(&self) -> [u8; 4] {
			// bitcoin network (xpub) (for test vectors)
			[0x04, 0x88, 0xB2, 0x1E]
		}
		fn master_seed() -> [u8; 12] {
			b"Bitcoin seed".to_owned()
		}
		fn init_sha512(&mut self, seed: &[u8]) {
			self.hmac_sha512 =
				HmacSha512::new_from_slice(seed).expect("HMAC can take key of any size");
		}
		fn append_sha512(&mut self, value: &[u8]) {
			self.hmac_sha512.update(value);
		}
		fn result_sha512(&mut self) -> [u8; 64] {
			let mut result = [0; 64];
			result.copy_from_slice(&self.hmac_sha512.to_owned().finalize().into_bytes());
			result
		}
		fn sha_256(&self, input: &[u8]) -> [u8; 32] {
			let mut sha2_res = [0; 32];
			let mut sha2 = Sha256::new();
			sha2.update(input);
			sha2_res.copy_from_slice(sha2.finalize().as_slice());
			sha2_res
		}
		fn ripemd_160(&self, input: &[u8]) -> [u8; 20] {
			let mut ripemd_res = [0; 20];
			let mut ripemd = Ripemd160::new();
			ripemd.update(input);
			ripemd_res.copy_from_slice(ripemd.finalize().as_slice());
			ripemd_res
		}
	}

	fn test_path(
		secp: &Secp256k1,
		seed: &[u8],
		path: &[ChildNumber],
		expected_sk: &str,
		expected_pk: &str,
	) {
		let mut h = BIP32ReferenceHasher::new();
		let mut sk = ExtendedPrivKey::new_master(secp, &mut h, seed).unwrap();
		let mut pk = ExtendedPubKey::from_private::<BIP32ReferenceHasher>(secp, &sk, &mut h);

		// Check derivation convenience method for ExtendedPrivKey
		assert_eq!(
			&sk.derive_priv(secp, &mut h, path).unwrap().to_string()[..],
			expected_sk
		);

		// Check derivation convenience method for ExtendedPubKey, should error
		// appropriately if any ChildNumber is hardened
		if path.iter().any(|cnum| cnum.is_hardened()) {
			assert_eq!(
				pk.derive_pub(secp, &mut h, path),
				Err(Error::CannotDeriveFromHardenedKey)
			);
		} else {
			assert_eq!(
				&pk.derive_pub(secp, &mut h, path).unwrap().to_string()[..],
				expected_pk
			);
		}

		// Derive keys, checking hardened and non-hardened derivation one-by-one
		for &num in path.iter() {
			sk = sk.ckd_priv(secp, &mut h, num).unwrap();
			match num {
				ChildNumber::Normal { .. } => {
					let pk2 = pk.ckd_pub(secp, &mut h, num).unwrap();
					pk = ExtendedPubKey::from_private::<BIP32ReferenceHasher>(secp, &sk, &mut h);
					assert_eq!(pk, pk2);
				}
				ChildNumber::Hardened { .. } => {
					assert_eq!(
						pk.ckd_pub(secp, &mut h, num),
						Err(Error::CannotDeriveFromHardenedKey)
					);
					pk = ExtendedPubKey::from_private::<BIP32ReferenceHasher>(secp, &sk, &mut h);
				}
			}
		}

		// Check result against expected base58
		assert_eq!(&sk.to_string()[..], expected_sk);
		assert_eq!(&pk.to_string()[..], expected_pk);
		// Check decoded base58 against result
		let decoded_sk = ExtendedPrivKey::from_str(expected_sk);
		let decoded_pk = ExtendedPubKey::from_str(expected_pk);
		assert_eq!(Ok(sk), decoded_sk);
		assert_eq!(Ok(pk), decoded_pk);
	}

	#[test]
	fn test_vector_1() {
		let secp = Secp256k1::new();
		let seed = from_hex("000102030405060708090a0b0c0d0e0f").unwrap();

		// m
		test_path(&secp, &seed, &[],
                  "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi",
                  "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8");

		// m/0h
		test_path(&secp, &seed, &[ChildNumber::from_hardened_idx(0)],
                  "xprv9uHRZZhk6KAJC1avXpDAp4MDc3sQKNxDiPvvkX8Br5ngLNv1TxvUxt4cV1rGL5hj6KCesnDYUhd7oWgT11eZG7XnxHrnYeSvkzY7d2bhkJ7",
                  "xpub68Gmy5EdvgibQVfPdqkBBCHxA5htiqg55crXYuXoQRKfDBFA1WEjWgP6LHhwBZeNK1VTsfTFUHCdrfp1bgwQ9xv5ski8PX9rL2dZXvgGDnw");

		// m/0h/1
		test_path(&secp, &seed, &[ChildNumber::from_hardened_idx(0), ChildNumber::from_normal_idx(1)],
                   "xprv9wTYmMFdV23N2TdNG573QoEsfRrWKQgWeibmLntzniatZvR9BmLnvSxqu53Kw1UmYPxLgboyZQaXwTCg8MSY3H2EU4pWcQDnRnrVA1xe8fs",
                   "xpub6ASuArnXKPbfEwhqN6e3mwBcDTgzisQN1wXN9BJcM47sSikHjJf3UFHKkNAWbWMiGj7Wf5uMash7SyYq527Hqck2AxYysAA7xmALppuCkwQ");

		// m/0h/1/2h
		test_path(&secp, &seed, &[ChildNumber::from_hardened_idx(0), ChildNumber::from_normal_idx(1), ChildNumber::from_hardened_idx(2)],
                  "xprv9z4pot5VBttmtdRTWfWQmoH1taj2axGVzFqSb8C9xaxKymcFzXBDptWmT7FwuEzG3ryjH4ktypQSAewRiNMjANTtpgP4mLTj34bhnZX7UiM",
                  "xpub6D4BDPcP2GT577Vvch3R8wDkScZWzQzMMUm3PWbmWvVJrZwQY4VUNgqFJPMM3No2dFDFGTsxxpG5uJh7n7epu4trkrX7x7DogT5Uv6fcLW5");

		// m/0h/1/2h/2
		test_path(&secp, &seed, &[ChildNumber::from_hardened_idx(0), ChildNumber::from_normal_idx(1), ChildNumber::from_hardened_idx(2), ChildNumber::from_normal_idx(2)],
                  "xprvA2JDeKCSNNZky6uBCviVfJSKyQ1mDYahRjijr5idH2WwLsEd4Hsb2Tyh8RfQMuPh7f7RtyzTtdrbdqqsunu5Mm3wDvUAKRHSC34sJ7in334",
                  "xpub6FHa3pjLCk84BayeJxFW2SP4XRrFd1JYnxeLeU8EqN3vDfZmbqBqaGJAyiLjTAwm6ZLRQUMv1ZACTj37sR62cfN7fe5JnJ7dh8zL4fiyLHV");

		// m/0h/1/2h/2/1000000000
		test_path(&secp, &seed, &[ChildNumber::from_hardened_idx(0), ChildNumber::from_normal_idx(1), ChildNumber::from_hardened_idx(2), ChildNumber::from_normal_idx(2), ChildNumber::from_normal_idx(1000000000)],
                  "xprvA41z7zogVVwxVSgdKUHDy1SKmdb533PjDz7J6N6mV6uS3ze1ai8FHa8kmHScGpWmj4WggLyQjgPie1rFSruoUihUZREPSL39UNdE3BBDu76",
                  "xpub6H1LXWLaKsWFhvm6RVpEL9P4KfRZSW7abD2ttkWP3SSQvnyA8FSVqNTEcYFgJS2UaFcxupHiYkro49S8yGasTvXEYBVPamhGW6cFJodrTHy");
	}

	#[test]
	fn test_vector_2() {
		let secp = Secp256k1::new();
		let seed = from_hex("fffcf9f6f3f0edeae7e4e1dedbd8d5d2cfccc9c6c3c0bdbab7b4b1aeaba8a5a29f9c999693908d8a8784817e7b7875726f6c696663605d5a5754514e4b484542").unwrap();

		// m
		test_path(&secp, &seed, &[],
                  "xprv9s21ZrQH143K31xYSDQpPDxsXRTUcvj2iNHm5NUtrGiGG5e2DtALGdso3pGz6ssrdK4PFmM8NSpSBHNqPqm55Qn3LqFtT2emdEXVYsCzC2U",
                  "xpub661MyMwAqRbcFW31YEwpkMuc5THy2PSt5bDMsktWQcFF8syAmRUapSCGu8ED9W6oDMSgv6Zz8idoc4a6mr8BDzTJY47LJhkJ8UB7WEGuduB");

		// m/0
		test_path(&secp, &seed, &[ChildNumber::from_normal_idx(0)],
                  "xprv9vHkqa6EV4sPZHYqZznhT2NPtPCjKuDKGY38FBWLvgaDx45zo9WQRUT3dKYnjwih2yJD9mkrocEZXo1ex8G81dwSM1fwqWpWkeS3v86pgKt",
                  "xpub69H7F5d8KSRgmmdJg2KhpAK8SR3DjMwAdkxj3ZuxV27CprR9LgpeyGmXUbC6wb7ERfvrnKZjXoUmmDznezpbZb7ap6r1D3tgFxHmwMkQTPH");

		// m/0/2147483647h
		test_path(&secp, &seed, &[ChildNumber::from_normal_idx(0), ChildNumber::from_hardened_idx(2147483647)],
                  "xprv9wSp6B7kry3Vj9m1zSnLvN3xH8RdsPP1Mh7fAaR7aRLcQMKTR2vidYEeEg2mUCTAwCd6vnxVrcjfy2kRgVsFawNzmjuHc2YmYRmagcEPdU9",
                  "xpub6ASAVgeehLbnwdqV6UKMHVzgqAG8Gr6riv3Fxxpj8ksbH9ebxaEyBLZ85ySDhKiLDBrQSARLq1uNRts8RuJiHjaDMBU4Zn9h8LZNnBC5y4a");

		// m/0/2147483647h/1
		test_path(&secp, &seed, &[ChildNumber::from_normal_idx(0), ChildNumber::from_hardened_idx(2147483647), ChildNumber::from_normal_idx(1)],
                  "xprv9zFnWC6h2cLgpmSA46vutJzBcfJ8yaJGg8cX1e5StJh45BBciYTRXSd25UEPVuesF9yog62tGAQtHjXajPPdbRCHuWS6T8XA2ECKADdw4Ef",
                  "xpub6DF8uhdarytz3FWdA8TvFSvvAh8dP3283MY7p2V4SeE2wyWmG5mg5EwVvmdMVCQcoNJxGoWaU9DCWh89LojfZ537wTfunKau47EL2dhHKon");

		// m/0/2147483647h/1/2147483646h
		test_path(&secp, &seed, &[ChildNumber::from_normal_idx(0), ChildNumber::from_hardened_idx(2147483647), ChildNumber::from_normal_idx(1), ChildNumber::from_hardened_idx(2147483646)],
                  "xprvA1RpRA33e1JQ7ifknakTFpgNXPmW2YvmhqLQYMmrj4xJXXWYpDPS3xz7iAxn8L39njGVyuoseXzU6rcxFLJ8HFsTjSyQbLYnMpCqE2VbFWc",
                  "xpub6ERApfZwUNrhLCkDtcHTcxd75RbzS1ed54G1LkBUHQVHQKqhMkhgbmJbZRkrgZw4koxb5JaHWkY4ALHY2grBGRjaDMzQLcgJvLJuZZvRcEL");

		// m/0/2147483647h/1/2147483646h/2
		test_path(&secp, &seed, &[ChildNumber::from_normal_idx(0), ChildNumber::from_hardened_idx(2147483647), ChildNumber::from_normal_idx(1), ChildNumber::from_hardened_idx(2147483646), ChildNumber::from_normal_idx(2)],
                  "xprvA2nrNbFZABcdryreWet9Ea4LvTJcGsqrMzxHx98MMrotbir7yrKCEXw7nadnHM8Dq38EGfSh6dqA9QWTyefMLEcBYJUuekgW4BYPJcr9E7j",
                  "xpub6FnCn6nSzZAw5Tw7cgR9bi15UV96gLZhjDstkXXxvCLsUXBGXPdSnLFbdpq8p9HmGsApME5hQTZ3emM2rnY5agb9rXpVGyy3bdW6EEgAtqt");
	}

	#[test]
	fn test_vector_3() {
		let secp = Secp256k1::new();
		let seed = from_hex("4b381541583be4423346c643850da4b320e46a87ae3d2a4e6da11eba819cd4acba45d239319ac14f863b8d5ab5a0d0c64d2e8a1e7d1457df2e5a3c51c73235be").unwrap();

		// m
		test_path(&secp, &seed, &[],
                  "xprv9s21ZrQH143K25QhxbucbDDuQ4naNntJRi4KUfWT7xo4EKsHt2QJDu7KXp1A3u7Bi1j8ph3EGsZ9Xvz9dGuVrtHHs7pXeTzjuxBrCmmhgC6",
                  "xpub661MyMwAqRbcEZVB4dScxMAdx6d4nFc9nvyvH3v4gJL378CSRZiYmhRoP7mBy6gSPSCYk6SzXPTf3ND1cZAceL7SfJ1Z3GC8vBgp2epUt13");

		// m/0h
		test_path(&secp, &seed, &[ChildNumber::from_hardened_idx(0)],
                  "xprv9uPDJpEQgRQfDcW7BkF7eTya6RPxXeJCqCJGHuCJ4GiRVLzkTXBAJMu2qaMWPrS7AANYqdq6vcBcBUdJCVVFceUvJFjaPdGZ2y9WACViL4L",
                  "xpub68NZiKmJWnxxS6aaHmn81bvJeTESw724CRDs6HbuccFQN9Ku14VQrADWgqbhhTHBaohPX4CjNLf9fq9MYo6oDaPPLPxSb7gwQN3ih19Zm4Y");
	}

	#[test]
	#[cfg(all(feature = "serde", feature = "strason"))]
	pub fn encode_decode_childnumber() {
		serde_round_trip!(ChildNumber::from_normal_idx(0));
		serde_round_trip!(ChildNumber::from_normal_idx(1));
		serde_round_trip!(ChildNumber::from_normal_idx((1 << 31) - 1));
		serde_round_trip!(ChildNumber::from_hardened_idx(0));
		serde_round_trip!(ChildNumber::from_hardened_idx(1));
		serde_round_trip!(ChildNumber::from_hardened_idx((1 << 31) - 1));
	}
}
