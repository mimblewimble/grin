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

use rand::{thread_rng, Rng};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::{error, fmt};

use util::secp;
use util::secp::{Message, Secp256k1, Signature};
use util::secp::key::SecretKey;
use util::secp::pedersen::Commitment;
use util::logger::LOGGER;
use blake2;
use blind::{BlindSum, BlindingFactor};
use extkey::{self, Identifier};

#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Error {
	ExtendedKey(extkey::Error),
	Secp(secp::Error),
	KeyDerivation(String),
	Transaction(String),
	RangeProof(String),
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

impl error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			_ => "some kind of keychain error",
		}
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			_ => write!(f, "some kind of keychain error"),
		}
	}
}

#[derive(Clone, Debug)]
pub struct Keychain {
	secp: Secp256k1,
	extkey: extkey::ExtendedKey,
	key_overrides: HashMap<Identifier, SecretKey>,
	key_derivation_cache: Arc<RwLock<HashMap<Identifier, u32>>>,
}

impl Keychain {
	pub fn root_key_id(&self) -> Identifier {
		self.extkey.root_key_id.clone()
	}

	// For tests and burn only, associate a key identifier with a known secret key.
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
			key_derivation_cache: Arc::new(RwLock::new(HashMap::new())),
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
		let child_key = self.extkey.derive(&self.secp, derivation)?;
		Ok(child_key.key_id)
	}

	pub fn derived_key(&self, key_id: &Identifier) -> Result<SecretKey, Error> {
		// first check our overrides and just return the key if we have one in there
		if let Some(key) = self.key_overrides.get(key_id) {
			trace!(
				LOGGER,
				"... Derived Key (using override) key_id: {}",
				key_id
			);
			return Ok(*key);
		}

		let child_key = self.derived_child_key(key_id)?;
		Ok(child_key.key)
	}

	fn derived_child_key(&self, key_id: &Identifier) -> Result<extkey::ChildKey, Error> {
		trace!(LOGGER, "Derived Key by key_id: {}", key_id);

		// then check the derivation cache to see if we have previously derived this key
		// if so use the derivation from the cache to derive the key
		{
			let cache = self.key_derivation_cache.read().unwrap();
			if let Some(derivation) = cache.get(key_id) {
				trace!(
					LOGGER,
					"... Derived Key (cache hit) key_id: {}, derivation: {}",
					key_id,
					derivation
				);
				return Ok(self.derived_key_from_index(*derivation)?);
			}
		}

		// otherwise iterate over a large number of derivations looking for our key
		// cache the resulting derivations by key_id for faster lookup later
		// TODO - remove hard limit (within reason)
		// TODO - do we benefit here if we track our max known n_child?
		{
			let mut cache = self.key_derivation_cache.write().unwrap();
			for i in 1..100_000 {
				let child_key = self.extkey.derive(&self.secp, i)?;
				// let child_key_id = extkey.identifier(&self.secp)?;

				if !cache.contains_key(&child_key.key_id) {
					trace!(
						LOGGER,
						"... Derived Key (cache miss) key_id: {}, derivation: {}",
						child_key.key_id,
						child_key.n_child,
					);
					cache.insert(child_key.key_id.clone(), child_key.n_child);
				}

				if child_key.key_id == *key_id {
					return Ok(child_key);
				}
			}
		}

		Err(Error::KeyDerivation(format!(
			"failed to derive child_key for {:?}",
			key_id
		)))
	}

	// if we know the derivation index we can just straight to deriving the key
	fn derived_key_from_index(&self, derivation: u32) -> Result<extkey::ChildKey, Error> {
		trace!(LOGGER, "Derived Key (fast) by derivation: {}", derivation);
		let child_key = self.extkey.derive(&self.secp, derivation)?;
		return Ok(child_key);
	}

	pub fn commit(&self, amount: u64, key_id: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(key_id)?;
		let commit = self.secp.commit(amount, skey)?;
		Ok(commit)
	}

	pub fn commit_with_key_index(&self, amount: u64, derivation: u32) -> Result<Commitment, Error> {
		let child_key = self.derived_key_from_index(derivation)?;
		let commit = self.secp.commit(amount, child_key.key)?;
		Ok(commit)
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
			.filter_map(|b| b.secret_key(&self.secp).ok())
			.collect::<Vec<SecretKey>>());

		neg_keys.extend(&blind_sum
			.negative_blinding_factors
			.iter()
			.filter_map(|b| b.secret_key(&self.secp).ok())
			.collect::<Vec<SecretKey>>());

		let sum = self.secp.blind_sum(pos_keys, neg_keys)?;
		Ok(BlindingFactor::from_secret_key(sum))
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
		let skey = &blinding.secret_key(&self.secp)?;
		let sig = self.secp.sign(msg, &skey)?;
		Ok(sig)
	}

	pub fn secp(&self) -> &Secp256k1 {
		&self.secp
	}
}

#[cfg(test)]
mod test {
	use keychain::{BlindSum, BlindingFactor, Keychain};
	use util::secp;
	use util::secp::key::SecretKey;

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

	// We plan to "offset" the key used in the kernel commitment
	// so we are going to be doing some key addition/subtraction.
	// This test is mainly to demonstrate that idea that summing commitments
	// and summing the keys used to commit to 0 have the same result.
	#[test]
	fn secret_key_addition() {
		let keychain = Keychain::from_random_seed().unwrap();

		let skey1 = SecretKey::from_slice(
			&keychain.secp,
			&[
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 1,
			],
		).unwrap();

		let skey2 = SecretKey::from_slice(
			&keychain.secp,
			&[
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 2,
			],
		).unwrap();

		// adding secret keys 1 and 2 to give secret key 3
		let mut skey3 = skey1.clone();
		let _ = skey3.add_assign(&keychain.secp, &skey2).unwrap();

		// create commitments for secret keys 1, 2 and 3
		// all committing to the value 0 (which is what we do for tx_kernels)
		let commit_1 = keychain.secp.commit(0, skey1).unwrap();
		let commit_2 = keychain.secp.commit(0, skey2).unwrap();
		let commit_3 = keychain.secp.commit(0, skey3).unwrap();

		// now sum commitments for keys 1 and 2
		let sum = keychain
			.secp
			.commit_sum(vec![commit_1.clone(), commit_2.clone()], vec![])
			.unwrap();

		// confirm the commitment for key 3 matches the sum of the commitments 1 and 2
		assert_eq!(sum, commit_3);

		// now check we can sum keys up using keychain.blind_sum()
		// in the same way (convenience function)
		assert_eq!(
			keychain
				.blind_sum(&BlindSum::new()
					.add_blinding_factor(BlindingFactor::from_secret_key(skey1))
					.add_blinding_factor(BlindingFactor::from_secret_key(skey2)))
				.unwrap(),
			BlindingFactor::from_secret_key(skey3),
		);
	}
}
