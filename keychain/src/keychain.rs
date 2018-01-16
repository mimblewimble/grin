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
use std::sync::{Arc, RwLock};

use util::secp;
use util::secp::{Message, Secp256k1, Signature};
use util::secp::key::{SecretKey, PublicKey};
use util::secp::pedersen::{Commitment, ProofMessage, ProofInfo, RangeProof};
use util::secp::aggsig;
use util::logger::LOGGER;
use util::kernel_sig_msg;
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

/// Holds internal information about an aggsig operation
#[derive(Clone, Debug)]
pub struct AggSigTxContext {
	// Secret key (of which public is shared)
	pub sec_key: SecretKey,
	// Secret nonce (of which public is shared)
	// (basically a SecretKey)
	pub sec_nonce: SecretKey,
	// If I'm the recipient, store my outputs between invocations (that I need to sum)
	pub output_ids: Vec<Identifier>,
}

#[derive(Clone, Debug)]
pub struct Keychain {
	secp: Secp256k1,
	extkey: extkey::ExtendedKey,
	pub aggsig_context: Arc<RwLock<Option<AggSigTxContext>>>,
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
			aggsig_context: Arc::new(RwLock::new(None)),
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
		let extkey = self.extkey.derive(&self.secp, derivation)?;
		let key_id = extkey.identifier(&self.secp)?;
		Ok(key_id)
	}

	fn derived_key(&self, key_id: &Identifier) -> Result<SecretKey, Error> {
		trace!(LOGGER, "Derived Key by key_id: {}", key_id);

		// first check our overrides and just return the key if we have one in there
		if let Some(key) = self.key_overrides.get(key_id) {
			trace!(LOGGER, "... Derived Key (using override) key_id: {}", key_id);
			return Ok(*key);
		}

		// then check the derivation cache to see if we have previously derived this key
		// if so use the derivation from the cache to derive the key
		{
			let cache = self.key_derivation_cache.read().unwrap();
			if let Some(derivation) = cache.get(key_id) {
				trace!(LOGGER, "... Derived Key (cache hit) key_id: {}, derivation: {}", key_id, derivation);
				return Ok(self.derived_key_from_index(*derivation)?)
			}
		}

		// otherwise iterate over a large number of derivations looking for our key
		// cache the resulting derivations by key_id for faster lookup later
		// TODO - remove hard limit (within reason)
		// TODO - do we benefit here if we track our max known n_child?
		{
			let mut cache = self.key_derivation_cache.write().unwrap();
			for i in 1..100_000 {
				let extkey = self.extkey.derive(&self.secp, i)?;
				let extkey_id = extkey.identifier(&self.secp)?;

				if !cache.contains_key(&extkey_id) {
					trace!(LOGGER, "... Derived Key (cache miss) key_id: {}, derivation: {}", extkey_id, extkey.n_child);
					cache.insert(extkey_id.clone(), extkey.n_child);
				}

				if extkey_id == *key_id {
					return Ok(extkey.key);
				}
			}
		}

		Err(Error::KeyDerivation(
			format!("cannot find extkey for {:?}", key_id),
		))
	}

	// if we know the derivation index we can just straight to deriving the key
	fn derived_key_from_index(
		&self,
		derivation: u32,
	) -> Result<SecretKey, Error> {
		trace!(LOGGER, "Derived Key (fast) by derivation: {}", derivation);
		let extkey = self.extkey.derive(&self.secp, derivation)?;
		return Ok(extkey.key)
	}

	pub fn commit(&self, amount: u64, key_id: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(key_id)?;
		let commit = self.secp.commit(amount, skey)?;
		Ok(commit)
	}

	pub fn commit_with_key_index(
		&self,
		amount: u64,
		derivation: u32,
	) -> Result<Commitment, Error> {
		let skey = self.derived_key_from_index(derivation)?;
		let commit = self.secp.commit(amount, skey)?;
		Ok(commit)
	}

	pub fn switch_commit(&self, key_id: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(key_id)?;
		let commit = self.secp.switch_commit(skey)?;
		Ok(commit)
	}

	pub fn switch_commit_from_index(&self, index:u32) -> Result<Commitment, Error> {
		//just do this directly, because cache seems really slow for wallet reconstruct
		let skey = self.extkey.derive(&self.secp, index)?;
		let skey = skey.key;
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

	pub fn aggsig_create_context(&self, sec_key:SecretKey) {
		let mut context = self.aggsig_context.write().unwrap();
		*context = Some(AggSigTxContext{
			sec_key: sec_key,
			sec_nonce: aggsig::export_secnonce_single(&self.secp).unwrap(),
			output_ids: vec![],
		});
	}

	/// Tracks an output contributing to my excess value (if it needs to
	/// be kept between invocations
	pub fn aggsig_add_output(&self, id: &Identifier){
		let mut agg_context=self.aggsig_context.write().unwrap();
		let agg_context_write=agg_context.as_mut().unwrap();
		agg_context_write.output_ids.push(id.clone());
	}

	/// Returns all stored outputs
	pub fn aggsig_get_outputs(&self) -> Vec<Identifier> {
		let context = self.aggsig_context.clone();
		let context_read=context.read().unwrap();
		let agg_context=context_read.as_ref().unwrap();
		agg_context.output_ids.clone()
	}

	/// Returns private key, private nonce
	pub fn aggsig_get_private_keys(&self) -> (SecretKey, SecretKey) {
		let context = self.aggsig_context.clone();
		let context_read=context.read().unwrap();
		let agg_context=context_read.as_ref().unwrap();
		(agg_context.sec_key.clone(),
		agg_context.sec_nonce.clone())
	}

	/// Returns public key, public nonce
	pub fn aggsig_get_public_keys(&self) -> (PublicKey, PublicKey) {
		let context = self.aggsig_context.clone();
		let context_read=context.read().unwrap();
		let agg_context=context_read.as_ref().unwrap();
		(PublicKey::from_secret_key(&self.secp, &agg_context.sec_key).unwrap(),
		PublicKey::from_secret_key(&self.secp, &agg_context.sec_nonce).unwrap())
	}

	/// Note 'secnonce' here is used to perform the signature, while 'pubnonce' just allows you to
	/// provide a custom public nonce to include while calculating e
	/// nonce_sum is the sum used to decide whether secnonce should be inverted during sig time
	pub fn aggsig_sign_single(&self, msg: &Message, secnonce:Option<&SecretKey>, pubnonce: Option<&PublicKey>, nonce_sum: Option<&PublicKey>) -> Result<Signature, Error> {
		let context = self.aggsig_context.clone();
		let context_read=context.read().unwrap();
		let agg_context=context_read.as_ref().unwrap();
		let sig = aggsig::sign_single(&self.secp, msg, &agg_context.sec_key, secnonce, pubnonce, nonce_sum)?;
		Ok(sig)
	}

	//Verifies an aggsig signature
	pub fn aggsig_verify_single(&self, sig: &Signature, msg: &Message, pubnonce:Option<&PublicKey>, pubkey:&PublicKey, is_partial:bool) -> bool {
		aggsig::verify_single(&self.secp, sig, msg, pubnonce, pubkey, is_partial)
	}

	//Verifies other final sig corresponds with what we're expecting
	pub fn aggsig_verify_final_sig_build_msg(&self, sig: &Signature, pubkey: &PublicKey, fee: u64, lock_height:u64) -> bool {
		let msg = secp::Message::from_slice(&kernel_sig_msg(fee, lock_height)).unwrap();
		self.aggsig_verify_single(sig, &msg, None, pubkey, true)
	}

	//Verifies other party's sig corresponds with what we're expecting
	pub fn aggsig_verify_partial_sig(&self, sig: &Signature, other_pub_nonce:&PublicKey, pubkey:&PublicKey, fee: u64, lock_height:u64) -> bool {
		let (_, sec_nonce) = self.aggsig_get_private_keys();
		let mut nonce_sum = other_pub_nonce.clone();
		let _ = nonce_sum.add_exp_assign(&self.secp, &sec_nonce);
		let msg = secp::Message::from_slice(&kernel_sig_msg(fee, lock_height)).unwrap();

		self.aggsig_verify_single(sig, &msg, Some(&nonce_sum), pubkey, true)
	}

	pub fn aggsig_calculate_partial_sig(&self, other_pub_nonce:&PublicKey, fee:u64, lock_height:u64) -> Result<Signature, Error>{
		// Add public nonces kR*G + kS*G
		let (_, sec_nonce) = self.aggsig_get_private_keys();
		let mut nonce_sum = other_pub_nonce.clone();
		let _ = nonce_sum.add_exp_assign(&self.secp, &sec_nonce);
		let msg = secp::Message::from_slice(&kernel_sig_msg(fee, lock_height))?;

		//Now calculate signature using message M=fee, nonce in e=nonce_sum
		self.aggsig_sign_single(&msg, Some(&sec_nonce), Some(&nonce_sum), Some(&nonce_sum))
	}

	/// Helper function to calculate final singature
	pub fn aggsig_calculate_final_sig(&self, their_sig: &Signature, our_sig: &Signature, their_pub_nonce: &PublicKey) -> Result<Signature, Error> {
		// Add public nonces kR*G + kS*G
		let (_, sec_nonce) = self.aggsig_get_private_keys();
		let mut nonce_sum = their_pub_nonce.clone();
		let _ = nonce_sum.add_exp_assign(&self.secp, &sec_nonce);
		let sig = aggsig::add_signatures_single(&self.secp, their_sig, our_sig, &nonce_sum)?;
		Ok(sig)
	}

	/// Helper function to calculate final public key
	pub fn aggsig_calculate_final_pubkey(
		&self,
		their_public_key: &PublicKey,
	) -> Result<PublicKey, Error> {
		let (our_sec_key, _) = self.aggsig_get_private_keys();
		let mut pk_sum = their_public_key.clone();
		let _ = pk_sum.add_exp_assign(&self.secp, &our_sec_key);
		Ok(pk_sum)
	}

	/// Just a simple sig, creates its own nonce, etc
	pub fn aggsig_sign_from_key_id(
		&self,
		msg: &Message,
		key_id: &Identifier,
	) -> Result<Signature, Error> {
		let skey = self.derived_key(key_id)?;
		let sig = aggsig::sign_single(&self.secp, &msg, &skey, None, None, None)?;
		Ok(sig)
	}

	/// Verifies a sig given a commitment
	pub fn aggsig_verify_single_from_commit(
		secp:&Secp256k1,
		sig: &Signature,
		msg: &Message,
		commit: &Commitment,
	) -> bool {
		// Extract the pubkey, unfortunately we need this hack for now, (we just hope one is valid)
		// TODO: Create better secp256k1 API to do this
		let pubkeys = commit.to_two_pubkeys(secp);
		let mut valid = false;
		for i in 0..pubkeys.len() {
			valid = aggsig::verify_single(secp, &sig, &msg, None, &pubkeys[i], false);
			if valid {
				break;
			}
		}
		valid
	}

	/// Just a simple sig, creates its own nonce, etc
	pub fn aggsig_sign_with_blinding(
		secp: &Secp256k1,
		msg: &Message,
		blinding: &BlindingFactor,
	) -> Result<Signature, Error> {
		let sig = aggsig::sign_single(secp, &msg, &blinding.secret_key(), None, None, None)?;
		Ok(sig)
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
