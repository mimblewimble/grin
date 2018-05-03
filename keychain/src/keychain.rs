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
use util::secp::key::{PublicKey, SecretKey};
use util::secp::pedersen::{Commitment, ProofInfo, ProofMessage, RangeProof};
use util::secp::aggsig;
use util::logger::LOGGER;
use util::kernel_sig_msg;
use blake2;
use uuid::Uuid;
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
	use rand::thread_rng;

	use uuid::Uuid;

	use keychain::{BlindSum, BlindingFactor, Keychain};
	use util::kernel_sig_msg;
	use util::secp;
	use util::secp::pedersen::ProofMessage;
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

	#[test]
	fn aggsig_sender_receiver_interaction() {
		let sender_keychain = Keychain::from_random_seed().unwrap();
		let receiver_keychain = Keychain::from_random_seed().unwrap();

		// tx identifier for wallet interaction
		let tx_id = Uuid::new_v4();

		// Calculate the kernel excess here for convenience.
		// Normally this would happen during transaction building.
		let kernel_excess = {
			let skey1 = sender_keychain
				.derived_key(&sender_keychain.derive_key_id(1).unwrap())
				.unwrap();

			let skey2 = receiver_keychain
				.derived_key(&receiver_keychain.derive_key_id(1).unwrap())
				.unwrap();

			let keychain = Keychain::from_random_seed().unwrap();
			let blinding_factor = keychain
				.blind_sum(&BlindSum::new()
					.sub_blinding_factor(BlindingFactor::from_secret_key(skey1))
					.add_blinding_factor(BlindingFactor::from_secret_key(skey2)))
				.unwrap();

			keychain
				.secp
				.commit(0, blinding_factor.secret_key(&keychain.secp).unwrap())
				.unwrap()
		};

		// sender starts the tx interaction
		let (sender_pub_excess, sender_pub_nonce) = {
			let keychain = sender_keychain.clone();

			let skey = keychain
				.derived_key(&keychain.derive_key_id(1).unwrap())
				.unwrap();

			// dealing with an input here so we need to negate the blinding_factor
			// rather than use it as is
			let bs = BlindSum::new();
			let blinding_factor = keychain
				.blind_sum(&bs.sub_blinding_factor(BlindingFactor::from_secret_key(skey)))
				.unwrap();

			let blind = blinding_factor.secret_key(&keychain.secp()).unwrap();

			keychain.aggsig_create_context(&tx_id, blind).unwrap();
			keychain.aggsig_get_public_keys(&tx_id)
		};

		// receiver receives partial tx
		let (receiver_pub_excess, receiver_pub_nonce, sig_part) = {
			let keychain = receiver_keychain.clone();
			let key_id = keychain.derive_key_id(1).unwrap();

			// let blind = blind_sum.secret_key(&keychain.secp())?;
			let blind = keychain.derived_key(&key_id).unwrap();

			keychain.aggsig_create_context(&tx_id, blind).unwrap();
			let (pub_excess, pub_nonce) = keychain.aggsig_get_public_keys(&tx_id);
			keychain.aggsig_add_output(&tx_id, &key_id);

			let sig_part = keychain
				.aggsig_calculate_partial_sig(&tx_id, &sender_pub_nonce, 0, 0)
				.unwrap();
			(pub_excess, pub_nonce, sig_part)
		};

		// check the sender can verify the partial signature
		// received in the response back from the receiver
		{
			let keychain = sender_keychain.clone();
			let sig_verifies = keychain.aggsig_verify_partial_sig(
				&tx_id,
				&sig_part,
				&receiver_pub_nonce,
				&receiver_pub_excess,
				0,
				0,
			);
			assert!(sig_verifies);
		}

		// now sender signs with their key
		let sender_sig_part = {
			let keychain = sender_keychain.clone();
			keychain
				.aggsig_calculate_partial_sig(&tx_id, &receiver_pub_nonce, 0, 0)
				.unwrap()
		};

		// check the receiver can verify the partial signature
		// received by the sender
		{
			let keychain = receiver_keychain.clone();
			let sig_verifies = keychain.aggsig_verify_partial_sig(
				&tx_id,
				&sender_sig_part,
				&sender_pub_nonce,
				&sender_pub_excess,
				0,
				0,
			);
			assert!(sig_verifies);
		}

		// Receiver now builds final signature from sender and receiver parts
		let (final_sig, final_pubkey) = {
			let keychain = receiver_keychain.clone();

			// Receiver recreates their partial sig (we do not maintain state from earlier)
			let our_sig_part = keychain
				.aggsig_calculate_partial_sig(&tx_id, &sender_pub_nonce, 0, 0)
				.unwrap();

			// Receiver now generates final signature from the two parts
			let final_sig = keychain
				.aggsig_calculate_final_sig(
					&tx_id,
					&sender_sig_part,
					&our_sig_part,
					&sender_pub_nonce,
				)
				.unwrap();

			// Receiver calculates the final public key (to verify sig later)
			let final_pubkey = keychain
				.aggsig_calculate_final_pubkey(&tx_id, &sender_pub_excess)
				.unwrap();

			(final_sig, final_pubkey)
		};

		// Receiver checks the final signature verifies
		{
			let keychain = receiver_keychain.clone();

			// Receiver check the final signature verifies
			let sig_verifies =
				keychain.aggsig_verify_final_sig_build_msg(&final_sig, &final_pubkey, 0, 0);
			assert!(sig_verifies);
		}

		// Check we can verify the sig using the kernel excess
		{
			let keychain = Keychain::from_random_seed().unwrap();

			let msg = secp::Message::from_slice(&kernel_sig_msg(0, 0)).unwrap();

			let sig_verifies = Keychain::aggsig_verify_single_from_commit(
				&keychain.secp,
				&final_sig,
				&msg,
				&kernel_excess,
			);

			assert!(sig_verifies);
		}
	}

	#[test]
	fn aggsig_sender_receiver_interaction_offset() {
		let sender_keychain = Keychain::from_random_seed().unwrap();
		let receiver_keychain = Keychain::from_random_seed().unwrap();

		// tx identifier for wallet interaction
		let tx_id = Uuid::new_v4();

		// This is the kernel offset that we use to split the key
		// Summing these at the block level prevents the
		// kernels from being used to reconstruct (or identify) individual transactions
		let kernel_offset = SecretKey::new(&sender_keychain.secp(), &mut thread_rng());

		// Calculate the kernel excess here for convenience.
		// Normally this would happen during transaction building.
		let kernel_excess = {
			let skey1 = sender_keychain
				.derived_key(&sender_keychain.derive_key_id(1).unwrap())
				.unwrap();

			let skey2 = receiver_keychain
				.derived_key(&receiver_keychain.derive_key_id(1).unwrap())
				.unwrap();

			let keychain = Keychain::from_random_seed().unwrap();
			let blinding_factor = keychain
				.blind_sum(&BlindSum::new()
					.sub_blinding_factor(BlindingFactor::from_secret_key(skey1))
					.add_blinding_factor(BlindingFactor::from_secret_key(skey2))
					// subtract the kernel offset here like as would when
					// verifying a kernel signature
					.sub_blinding_factor(BlindingFactor::from_secret_key(kernel_offset)))
				.unwrap();

			keychain
				.secp
				.commit(0, blinding_factor.secret_key(&keychain.secp).unwrap())
				.unwrap()
		};

		// sender starts the tx interaction
		let (sender_pub_excess, sender_pub_nonce) = {
			let keychain = sender_keychain.clone();

			let skey = keychain
				.derived_key(&keychain.derive_key_id(1).unwrap())
				.unwrap();

			// dealing with an input here so we need to negate the blinding_factor
			// rather than use it as is
			let blinding_factor = keychain
				.blind_sum(&BlindSum::new()
					.sub_blinding_factor(BlindingFactor::from_secret_key(skey))
					// subtract the kernel offset to create an aggsig context
					// with our "split" key
					.sub_blinding_factor(BlindingFactor::from_secret_key(kernel_offset)))
				.unwrap();

			let blind = blinding_factor.secret_key(&keychain.secp()).unwrap();

			keychain.aggsig_create_context(&tx_id, blind).unwrap();
			keychain.aggsig_get_public_keys(&tx_id)
		};

		// receiver receives partial tx
		let (receiver_pub_excess, receiver_pub_nonce, sig_part) = {
			let keychain = receiver_keychain.clone();
			let key_id = keychain.derive_key_id(1).unwrap();

			let blind = keychain.derived_key(&key_id).unwrap();

			keychain.aggsig_create_context(&tx_id, blind).unwrap();
			let (pub_excess, pub_nonce) = keychain.aggsig_get_public_keys(&tx_id);
			keychain.aggsig_add_output(&tx_id, &key_id);

			let sig_part = keychain
				.aggsig_calculate_partial_sig(&tx_id, &sender_pub_nonce, 0, 0)
				.unwrap();
			(pub_excess, pub_nonce, sig_part)
		};

		// check the sender can verify the partial signature
		// received in the response back from the receiver
		{
			let keychain = sender_keychain.clone();
			let sig_verifies = keychain.aggsig_verify_partial_sig(
				&tx_id,
				&sig_part,
				&receiver_pub_nonce,
				&receiver_pub_excess,
				0,
				0,
			);
			assert!(sig_verifies);
		}

		// now sender signs with their key
		let sender_sig_part = {
			let keychain = sender_keychain.clone();
			keychain
				.aggsig_calculate_partial_sig(&tx_id, &receiver_pub_nonce, 0, 0)
				.unwrap()
		};

		// check the receiver can verify the partial signature
		// received by the sender
		{
			let keychain = receiver_keychain.clone();
			let sig_verifies = keychain.aggsig_verify_partial_sig(
				&tx_id,
				&sender_sig_part,
				&sender_pub_nonce,
				&sender_pub_excess,
				0,
				0,
			);
			assert!(sig_verifies);
		}

		// Receiver now builds final signature from sender and receiver parts
		let (final_sig, final_pubkey) = {
			let keychain = receiver_keychain.clone();

			// Receiver recreates their partial sig (we do not maintain state from earlier)
			let our_sig_part = keychain
				.aggsig_calculate_partial_sig(&tx_id, &sender_pub_nonce, 0, 0)
				.unwrap();

			// Receiver now generates final signature from the two parts
			let final_sig = keychain
				.aggsig_calculate_final_sig(
					&tx_id,
					&sender_sig_part,
					&our_sig_part,
					&sender_pub_nonce,
				)
				.unwrap();

			// Receiver calculates the final public key (to verify sig later)
			let final_pubkey = keychain
				.aggsig_calculate_final_pubkey(&tx_id, &sender_pub_excess)
				.unwrap();

			(final_sig, final_pubkey)
		};

		// Receiver checks the final signature verifies
		{
			let keychain = receiver_keychain.clone();

			// Receiver check the final signature verifies
			let sig_verifies =
				keychain.aggsig_verify_final_sig_build_msg(&final_sig, &final_pubkey, 0, 0);
			assert!(sig_verifies);
		}

		// Check we can verify the sig using the kernel excess
		{
			let keychain = Keychain::from_random_seed().unwrap();

			let msg = secp::Message::from_slice(&kernel_sig_msg(0, 0)).unwrap();

			let sig_verifies = Keychain::aggsig_verify_single_from_commit(
				&keychain.secp,
				&final_sig,
				&msg,
				&kernel_excess,
			);

			assert!(sig_verifies);
		}
	}

	#[test]
	fn test_rewind_range_proof() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let commit = keychain.commit(5, &key_id).unwrap();
		let msg = ProofMessage::from_bytes(&[0u8; 64]);
		let extra_data = [99u8; 64];

		let proof = keychain
			.range_proof(5, &key_id, commit, Some(extra_data.to_vec().clone()), msg)
			.unwrap();
		let proof_info = keychain
			.rewind_range_proof(&key_id, commit, Some(extra_data.to_vec().clone()), proof)
			.unwrap();

		assert_eq!(proof_info.success, true);

		// now check the recovered message is "empty" (but not truncated) i.e. all
		// zeroes
		//Value is in the message in this case
		assert_eq!(
			proof_info.message,
			secp::pedersen::ProofMessage::from_bytes(&[0; secp::constants::BULLET_PROOF_MSG_SIZE])
		);

		let key_id2 = keychain.derive_key_id(2).unwrap();

		// cannot rewind with a different nonce
		let proof_info = keychain
			.rewind_range_proof(&key_id2, commit, Some(extra_data.to_vec().clone()), proof)
			.unwrap();
		// With bullet proofs, if you provide the wrong nonce you'll get gibberish back
		// as opposed to a failure to recover the message
		assert_ne!(
			proof_info.message,
			secp::pedersen::ProofMessage::from_bytes(&[0; secp::constants::BULLET_PROOF_MSG_SIZE])
		);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with a commitment to the same value using a different key
		let commit2 = keychain.commit(5, &key_id2).unwrap();
		let proof_info = keychain
			.rewind_range_proof(&key_id, commit2, Some(extra_data.to_vec().clone()), proof)
			.unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with a commitment to a different value
		let commit3 = keychain.commit(4, &key_id).unwrap();
		let proof_info = keychain
			.rewind_range_proof(&key_id, commit3, Some(extra_data.to_vec().clone()), proof)
			.unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with wrong extra committed data
		let commit3 = keychain.commit(4, &key_id).unwrap();
		let wrong_extra_data = [98u8; 64];
		let _should_err = keychain
			.rewind_range_proof(
				&key_id,
				commit3,
				Some(wrong_extra_data.to_vec().clone()),
				proof,
			)
			.unwrap();

		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);
	}
}
