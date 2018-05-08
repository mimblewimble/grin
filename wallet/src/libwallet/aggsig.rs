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
/// Aggsig library definitions

use std::collections::HashMap;

use util::secp::key::{PublicKey, SecretKey};
use util::secp::{self, aggsig, Message, Secp256k1, Signature};
use util::secp::pedersen::Commitment;
use util::kernel_sig_msg;
use uuid::Uuid;
use keychain::Keychain;
use keychain::extkey::Identifier;
use keychain::blind::BlindingFactor;
use libwallet::error::Error;

#[derive(Clone, Debug)]
/// Holds the context for a single aggsig transaction
pub struct Context {
	/// Transaction ID
	pub transaction_id: Uuid,
	/// Secret key (of which public is shared)
	pub sec_key: SecretKey,
	/// Secret nonce (of which public is shared)
	/// (basically a SecretKey)
	pub sec_nonce: SecretKey,
	/// If I'm the recipient, store my outputs between invocations (that I need
	/// to sum)
	pub output_ids: Vec<Identifier>,
}

#[derive(Clone, Debug)]
/// Holds many contexts, to support multiple transactions hitting a wallet receiver
/// at once
pub struct ContextManager {
	contexts: HashMap<Uuid, Context>,
}

impl ContextManager {
	/// Create
	pub fn new() -> ContextManager {
		ContextManager {
			contexts: HashMap::new(),
		}
	}

	/// Creates a context for a transaction id if required
	/// otherwise does nothing
	pub fn create_context(
		&mut self,
		secp: &secp::Secp256k1,
		transaction_id: &Uuid,
		sec_key: SecretKey,
	) -> Context {
		if !self.contexts.contains_key(transaction_id) {
			self.contexts.insert(
				transaction_id.clone(),
				Context {
					sec_key: sec_key,
					transaction_id: transaction_id.clone(),
					sec_nonce: aggsig::export_secnonce_single(secp).unwrap(),
					output_ids: vec![],
				},
			);
		}
		self.get_context(transaction_id)
	}

	/// Retrieve a context by transaction id
	pub fn get_context(&self, transaction_id: &Uuid) -> Context {
		self.contexts.get(&transaction_id).unwrap().clone()
	}

	/// Save context
	pub fn save_context(&mut self, c: Context) {
		self.contexts.insert(c.transaction_id.clone(), c);
	}
}

impl Context {
	/// Tracks an output contributing to my excess value (if it needs to
	/// be kept between invocations
	pub fn add_output(&mut self, output_id: &Identifier) {
		self.output_ids.push(output_id.clone());
	}

	/// Returns all stored outputs
	pub fn get_outputs(&self) -> Vec<Identifier> {
		self.output_ids.clone()
	}

	/// Returns private key, private nonce
	pub fn get_private_keys(&self) -> (SecretKey, SecretKey) {
		(self.sec_key.clone(), self.sec_nonce.clone())
	}

	/// Returns public key, public nonce
	pub fn get_public_keys(&self, secp: &Secp256k1) -> (PublicKey, PublicKey) {
		(
			PublicKey::from_secret_key(secp, &self.sec_key).unwrap(),
			PublicKey::from_secret_key(secp, &self.sec_nonce).unwrap(),
		)
	}

	/// Note 'secnonce' here is used to perform the signature, while 'pubnonce' just allows you to
	/// provide a custom public nonce to include while calculating e
	/// nonce_sum is the sum used to decide whether secnonce should be inverted during sig time
	pub fn sign_single(
		&self,
		secp: &Secp256k1,
		msg: &Message,
		secnonce: Option<&SecretKey>,
		pubnonce: Option<&PublicKey>,
		nonce_sum: Option<&PublicKey>,
	) -> Result<Signature, Error> {
		let sig = aggsig::sign_single(secp, msg, &self.sec_key, secnonce, pubnonce, nonce_sum)?;
		Ok(sig)
	}

	//Verifies other final sig corresponds with what we're expecting
	pub fn verify_final_sig_build_msg(
		&self,
		secp: &Secp256k1,
		sig: &Signature,
		pubkey: &PublicKey,
		fee: u64,
		lock_height: u64,
	) -> bool {
		let msg = secp::Message::from_slice(&kernel_sig_msg(fee, lock_height)).unwrap();
		verify_single(secp, sig, &msg, None, pubkey, true)
	}

	//Verifies other party's sig corresponds with what we're expecting
	pub fn verify_partial_sig(
		&self,
		secp: &Secp256k1,
		sig: &Signature,
		other_pub_nonce: &PublicKey,
		pubkey: &PublicKey,
		fee: u64,
		lock_height: u64,
	) -> bool {
		let (_, sec_nonce) = self.get_private_keys();
		let mut nonce_sum = other_pub_nonce.clone();
		let _ = nonce_sum.add_exp_assign(secp, &sec_nonce);
		let msg = secp::Message::from_slice(&kernel_sig_msg(fee, lock_height)).unwrap();

		verify_single(secp, sig, &msg, Some(&nonce_sum), pubkey, true)
	}

	pub fn calculate_partial_sig(
		&self,
		secp: &Secp256k1,
		other_pub_nonce: &PublicKey,
		fee: u64,
		lock_height: u64,
	) -> Result<Signature, Error> {
		// Add public nonces kR*G + kS*G
		let (_, sec_nonce) = self.get_private_keys();
		let mut nonce_sum = other_pub_nonce.clone();
		let _ = nonce_sum.add_exp_assign(secp, &sec_nonce);
		let msg = secp::Message::from_slice(&kernel_sig_msg(fee, lock_height))?;

		//Now calculate signature using message M=fee, nonce in e=nonce_sum
		self.sign_single(
			secp,
			&msg,
			Some(&sec_nonce),
			Some(&nonce_sum),
			Some(&nonce_sum),
		)
	}

	/// Helper function to calculate final signature
	pub fn calculate_final_sig(
		&self,
		secp: &Secp256k1,
		their_sig: &Signature,
		our_sig: &Signature,
		their_pub_nonce: &PublicKey,
	) -> Result<Signature, Error> {
		// Add public nonces kR*G + kS*G
		let (_, sec_nonce) = self.get_private_keys();
		let mut nonce_sum = their_pub_nonce.clone();
		let _ = nonce_sum.add_exp_assign(secp, &sec_nonce);
		let sig = aggsig::add_signatures_single(&secp, their_sig, our_sig, &nonce_sum)?;
		Ok(sig)
	}

	/// Helper function to calculate final public key
	pub fn calculate_final_pubkey(
		&self,
		secp: &Secp256k1,
		their_public_key: &PublicKey,
	) -> Result<PublicKey, Error> {
		let (our_sec_key, _) = self.get_private_keys();
		let mut pk_sum = their_public_key.clone();
		let _ = pk_sum.add_exp_assign(secp, &our_sec_key);
		Ok(pk_sum)
	}
}

// Contextless functions

/// Just a simple sig, creates its own nonce, etc
pub fn sign_from_key_id(
	secp: &Secp256k1,
	k: &Keychain,
	msg: &Message,
	key_id: &Identifier,
) -> Result<Signature, Error> {
	let skey = k.derived_key(key_id)?;
	let sig = aggsig::sign_single(secp, &msg, &skey, None, None, None)?;
	Ok(sig)
}

/// Verifies a sig given a commitment
pub fn verify_single_from_commit(
	secp: &Secp256k1,
	sig: &Signature,
	msg: &Message,
	commit: &Commitment,
) -> bool {
	// Extract the pubkey, unfortunately we need this hack for now, (we just hope
	// one is valid) TODO: Create better secp256k1 API to do this
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

//Verifies an aggsig signature
pub fn verify_single(
	secp: &Secp256k1,
	sig: &Signature,
	msg: &Message,
	pubnonce: Option<&PublicKey>,
	pubkey: &PublicKey,
	is_partial: bool,
) -> bool {
	aggsig::verify_single(secp, sig, msg, pubnonce, pubkey, is_partial)
}

/// Just a simple sig, creates its own nonce, etc
pub fn sign_with_blinding(
	secp: &Secp256k1,
	msg: &Message,
	blinding: &BlindingFactor,
) -> Result<Signature, Error> {
	let skey = &blinding.secret_key(&secp)?;
	let sig = aggsig::sign_single(secp, &msg, skey, None, None, None)?;
	Ok(sig)
}
