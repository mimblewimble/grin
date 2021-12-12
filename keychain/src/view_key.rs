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

use crate::blake2::blake2b::blake2b;
use byteorder::{BigEndian, ByteOrder};
//use crate::sha2::{Digest, Sha256};
use super::extkey_bip32::{
	BIP32Hasher, ChainCode, ChildNumber, Error as BIP32Error, ExtendedPrivKey, ExtendedPubKey,
	Fingerprint,
};
use super::types::{Error, Keychain};
use crate::util::secp::constants::GENERATOR_PUB_J_RAW;
use crate::util::secp::ffi;
use crate::util::secp::key::{PublicKey, SecretKey};
use crate::util::secp::Secp256k1;
use crate::SwitchCommitmentType;

/*const VERSION_TEST_NS: [u8;4] = [0x03, 0x27, 0x3E, 0x4B];
const VERSION_TEST: [u8;4]    = [0x03, 0x27, 0x3E, 0x4B];
const VERSION_MAIN_NS: [u8;4] = [0x03, 0x3C, 0x08, 0xDF];
const VERSION_MAIN: [u8;4]    = [0x03, 0x3C, 0x08, 0xDF];*/

/// Key that can be used to scan the chain for owned outputs
/// This is a public key, meaning it cannot be used to spend those outputs
/// At the moment only depth 0 keys can be used
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ViewKey {
	/// Whether this view key is meant for testnet or not
	pub is_test: bool,
	/// How many derivations this key is from the master (which is 0)
	pub depth: u8,
	/// Fingerprint of the parent key
	parent_fingerprint: Fingerprint,
	/// Child number of the key used to derive from parent (0 for master)
	pub child_number: ChildNumber,
	/// Public key
	public_key: PublicKey,
	/// Switch public key, required to view outputs that use switch commitment
	switch_public_key: Option<PublicKey>,
	/// Chain code
	chain_code: ChainCode,
	/// Hash used to generate rewind nonce
	pub rewind_hash: Vec<u8>,
}

impl ViewKey {
	pub fn create<K, H>(
		keychain: &K,
		ext_key: ExtendedPrivKey,
		hasher: &mut H,
		is_test: bool,
	) -> Result<Self, Error>
	where
		K: Keychain,
		H: BIP32Hasher,
	{
		let secp = keychain.secp();

		let ExtendedPubKey {
			network: _,
			depth,
			parent_fingerprint,
			child_number,
			public_key,
			chain_code,
		} = ExtendedPubKey::from_private(secp, &ext_key, hasher);

		let mut switch_public_key = PublicKey(ffi::PublicKey(GENERATOR_PUB_J_RAW));
		switch_public_key.mul_assign(secp, &ext_key.secret_key)?;
		let switch_public_key = Some(switch_public_key);

		let rewind_hash = Self::rewind_hash(secp, keychain.public_root_key());

		Ok(Self {
			is_test,
			depth,
			parent_fingerprint,
			child_number,
			public_key,
			switch_public_key,
			chain_code,
			rewind_hash,
		})
	}

	pub fn rewind_hash(secp: &Secp256k1, public_root_key: PublicKey) -> Vec<u8> {
		let ser = public_root_key.serialize_vec(secp, true);
		blake2b(32, &[], &ser[..]).as_bytes().to_vec()
	}

	fn ckd_pub_tweak<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		i: ChildNumber,
	) -> Result<(SecretKey, ChainCode), Error>
	where
		H: BIP32Hasher,
	{
		match i {
			ChildNumber::Hardened { .. } => Err(BIP32Error::CannotDeriveFromHardenedKey.into()),
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

	pub fn ckd_pub<H>(
		&self,
		secp: &Secp256k1,
		hasher: &mut H,
		i: ChildNumber,
	) -> Result<Self, Error>
	where
		H: BIP32Hasher,
	{
		let (secret_key, chain_code) = self.ckd_pub_tweak(secp, hasher, i)?;

		let mut public_key = self.public_key;
		public_key.add_exp_assign(secp, &secret_key)?;

		let switch_public_key = match &self.switch_public_key {
			Some(p) => {
				let mut j = PublicKey(ffi::PublicKey(GENERATOR_PUB_J_RAW));
				j.mul_assign(secp, &secret_key)?;
				Some(PublicKey::from_combination(secp, vec![p, &j])?)
			}
			None => None,
		};

		Ok(Self {
			is_test: self.is_test,
			depth: self.depth + 1,
			parent_fingerprint: self.fingerprint(secp, hasher),
			child_number: i,
			public_key,
			switch_public_key,
			chain_code,
			rewind_hash: self.rewind_hash.clone(),
		})
	}

	pub fn commit(
		&self,
		secp: &Secp256k1,
		amount: u64,
		switch: SwitchCommitmentType,
	) -> Result<PublicKey, Error> {
		let value_key = secp.commit_value(amount)?.to_pubkey(secp)?;
		let pub_key = PublicKey::from_combination(secp, vec![&self.public_key, &value_key])?;
		match switch {
			SwitchCommitmentType::None => Ok(pub_key),
			SwitchCommitmentType::Regular => {
				// TODO: replace this whole block by a libsecp function
				/*let switch_pub = self.switch_public_key.ok_or(Error::SwitchCommitment)?;
				let switch_ser: Vec<u8> = switch_pub.serialize_vec(secp, true)[..].to_vec();

				let mut commit_ser: Vec<u8> = pub_key.serialize_vec(secp, true)[..].to_vec();
				commit_ser[0] += 6; // This only works sometimes

				let mut hasher = Sha256::new();
				hasher.input(&commit_ser);
				hasher.input(&switch_ser);
				let blind = SecretKey::from_slice(secp, &hasher.result()[..])?;
				let mut pub_key = pub_key;
				pub_key.add_exp_assign(secp, &blind)?;

				Ok(pub_key)*/
				Err(Error::SwitchCommitment)
			}
		}
	}

	fn identifier<H>(&self, secp: &Secp256k1, hasher: &mut H) -> [u8; 20]
	where
		H: BIP32Hasher,
	{
		let sha2_res = hasher.sha_256(&self.public_key.serialize_vec(secp, true)[..]);
		hasher.ripemd_160(&sha2_res)
	}

	fn fingerprint<H>(&self, secp: &Secp256k1, hasher: &mut H) -> Fingerprint
	where
		H: BIP32Hasher,
	{
		Fingerprint::from(&self.identifier(secp, hasher)[0..4])
	}
}
