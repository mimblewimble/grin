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

use rand::{thread_rng, Rng};
use std::collections::HashMap;

use util::secp;
use util::secp::{Message, Secp256k1, Signature};
use util::secp::key::SecretKey;
use util::secp::pedersen::{Commitment, ProofMessage, ProofInfo, RangeProof};
use util::logger::LOGGER;
use blake2;
use blind::{BlindSum, BlindingFactor};
use extkey::{self, Identifier};


#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Error {
	ExtendedKey(extkey::Error),
	Secp(secp::Error),
	KeyDerivation(String),
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<extkey::Error> for Error {
	fn from(e: extkey::Error) -> Error {
		Error::ExtendedKey(e)
	}
}

#[derive(Clone, Debug)]
pub struct Keychain {
	secp: Secp256k1,
	extkey: extkey::ExtendedKey,
	key_overrides: HashMap<Identifier, SecretKey>,
}

impl Keychain {
	pub fn root_key_id(&self) -> Identifier {
		self.extkey.root_key_id.clone()
	}

	// For tests and burn only, associate a key identifier with a known secret key.
 //
	pub fn burn_enabled(keychain: &Keychain, burn_key_id: &Identifier) -> Keychain {
		let mut key_overrides = HashMap::new();
		key_overrides.insert(
			burn_key_id.clone(),
			SecretKey::from_slice(&keychain.secp, &[1; 32]).unwrap(),
		);
		Keychain {
			key_overrides: key_overrides,
			..keychain.clone()
		}
	}

	pub fn from_seed(seed: &[u8]) -> Result<Keychain, Error> {
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		let extkey = extkey::ExtendedKey::from_seed(&secp, seed)?;
		let keychain = Keychain {
			secp: secp,
			extkey: extkey,
			key_overrides: HashMap::new(),
		};
		Ok(keychain)
	}

	/// For testing - probably not a good idea to use outside of tests.
	pub fn from_random_seed() -> Result<Keychain, Error> {
		let seed: String = thread_rng().gen_ascii_chars().take(16).collect();
		let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());
		Keychain::from_seed(seed.as_bytes())
	}

	pub fn derive_key_id(&self, derivation: u32) -> Result<Identifier, Error> {
		let extkey = self.extkey.derive(&self.secp, derivation)?;
		let key_id = extkey.identifier(&self.secp)?;
		Ok(key_id)
	}

	fn derived_key_search(&self, key_id: &Identifier, n_child: Option<u32>) -> Result<SecretKey, Error> {
		if let Some(key) = self.key_overrides.get(key_id) {
			return Ok(*key);
		}

		trace!(LOGGER, "Derived Key key_id: {}", key_id);

		if let Some(n) = n_child{
			let extkey = self.extkey.derive(&self.secp, n)?;
			return Ok(extkey.key);
		};

		for i in 1..10000 {
			let extkey = self.extkey.derive(&self.secp, i)?;
			if extkey.identifier(&self.secp)? == *key_id {
				return Ok(extkey.key);
			}
		}
		Err(Error::KeyDerivation(
			format!("cannot find extkey for {:?}", key_id),
		))
	}

	fn derived_key(&self, key_id: &Identifier) -> Result<SecretKey, Error> {
		self.derived_key_search(key_id, None)
	}

	fn derived_key_from_index(&self, key_id: &Identifier, n_child:u32) -> Result<SecretKey, Error> {
		self.derived_key_search(key_id, Some(n_child))
	}

	pub fn commit(&self, amount: u64, key_id: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(key_id)?;
		let commit = self.secp.commit(amount, skey)?;
		Ok(commit)
	}

	pub fn commit_with_key_index(&self, amount: u64, key_id: &Identifier, n_child: u32) -> Result<Commitment, Error> {
		let skey = self.derived_key_from_index(key_id, n_child)?;
		let commit = self.secp.commit(amount, skey)?;
		Ok(commit)
	}

	pub fn switch_commit(&self, key_id: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(key_id)?;
		let commit = self.secp.switch_commit(skey)?;
		Ok(commit)
	}

	pub fn range_proof(
		&self,
		amount: u64,
		key_id: &Identifier,
		commit: Commitment,
		msg: ProofMessage,
	) -> Result<RangeProof, Error> {
		let skey = self.derived_key(key_id)?;
		let range_proof = self.secp.range_proof(0, amount, skey, commit, msg);
		Ok(range_proof)
	}

	pub fn rewind_range_proof(
		&self,
		key_id: &Identifier,
		commit: Commitment,
		proof: RangeProof,
	) -> Result<ProofInfo, Error> {
		let nonce = self.derived_key(key_id)?;
		Ok(self.secp.rewind_range_proof(commit, proof, nonce))
	}

	pub fn blind_sum(&self, blind_sum: &BlindSum) -> Result<BlindingFactor, Error> {
		let mut pos_keys: Vec<SecretKey> = blind_sum
			.positive_key_ids
			.iter()
			.filter_map(|k| self.derived_key(&k).ok())
			.collect();

		let mut neg_keys: Vec<SecretKey> = blind_sum
			.negative_key_ids
			.iter()
			.filter_map(|k| self.derived_key(&k).ok())
			.collect();

		pos_keys.extend(&blind_sum
			.positive_blinding_factors
			.iter()
			.map(|b| b.secret_key())
			.collect::<Vec<SecretKey>>());

		neg_keys.extend(&blind_sum
			.negative_blinding_factors
			.iter()
			.map(|b| b.secret_key())
			.collect::<Vec<SecretKey>>());

		let blinding = self.secp.blind_sum(pos_keys, neg_keys)?;
		Ok(BlindingFactor::new(blinding))
	}

	pub fn sign(&self, msg: &Message, key_id: &Identifier) -> Result<Signature, Error> {
		let skey = self.derived_key(key_id)?;
		let sig = self.secp.sign(msg, &skey)?;
		Ok(sig)
	}

	pub fn sign_with_blinding(
		&self,
		msg: &Message,
		blinding: &BlindingFactor,
	) -> Result<Signature, Error> {
		let sig = self.secp.sign(msg, &blinding.secret_key())?;
		Ok(sig)
	}

	pub fn secp(&self) -> &Secp256k1 {
		&self.secp
	}
}

#[cfg(test)]
mod test {
	use keychain::Keychain;
	use util::secp;
	use util::secp::pedersen::ProofMessage;

	#[test]
	fn test_key_derivation() {
		let keychain = Keychain::from_random_seed().unwrap();
		let secp = keychain.secp();

		// use the keychain to derive a "key_id" based on the underlying seed
		let key_id = keychain.derive_key_id(1).unwrap();

		let msg_bytes = [0; 32];
		let msg = secp::Message::from_slice(&msg_bytes[..]).unwrap();

		// now create a zero commitment using the key on the keychain associated with
  // the key_id
		let commit = keychain.commit(0, &key_id).unwrap();

		// now check we can use our key to verify a signature from this zero commitment
		let sig = keychain.sign(&msg, &key_id).unwrap();
		secp.verify_from_commit(&msg, &sig, &commit).unwrap();
	}

	#[test]
	fn test_rewind_range_proof() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();
		let msg = ProofMessage::empty();

		let proof = keychain.range_proof(5, &key_id, commit, msg).unwrap();
		let proof_info = keychain.rewind_range_proof(&key_id, commit, proof).unwrap();

		assert_eq!(proof_info.success, true);
		assert_eq!(proof_info.value, 5);

		// now check the recovered message is "empty" (but not truncated) i.e. all
  // zeroes
		assert_eq!(
			proof_info.message,
			secp::pedersen::ProofMessage::from_bytes(&[0; secp::constants::PROOF_MSG_SIZE])
		);

		let key_id2 = keychain.derive_key_id(2).unwrap();

		// cannot rewind with a different nonce
		let proof_info = keychain
			.rewind_range_proof(&key_id2, commit, proof)
			.unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with a commitment to the same value using a different key
		let commit2 = keychain.commit(5, &key_id2).unwrap();
		let proof_info = keychain
			.rewind_range_proof(&key_id, commit2, proof)
			.unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with a commitment to a different value
		let commit3 = keychain.commit(4, &key_id).unwrap();
		let proof_info = keychain
			.rewind_range_proof(&key_id, commit3, proof)
			.unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);
	}
}
