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

//! Aggregated Signature functions used in the creation of Grin transactions.
//! This module interfaces into the underlying
//! [Rust Aggsig library](https://github.com/mimblewimble/rust-secp256k1-zkp/blob/master/src/aggsig.rs)

use crate::libtx::error::Error;
use keychain::{BlindingFactor, Identifier, Keychain, SwitchCommitmentType};
use util::secp::key::{PublicKey, SecretKey};
use util::secp::pedersen::Commitment;
use util::secp::{self, aggsig, Message, Secp256k1, Signature};

/// Creates a new secure nonce (as a SecretKey), guaranteed to be usable during
/// aggsig creation.
///
/// # Arguments
///
/// * `secp` - A Secp256k1 Context initialized for Signing
///
/// # Example
///
/// ```
/// # extern crate grin_core as core;
/// use core::libtx::aggsig;
/// use util::secp::{ContextFlag, Secp256k1};
/// let secp = Secp256k1::with_caps(ContextFlag::SignOnly);
/// let secret_nonce = aggsig::create_secnonce(&secp).unwrap();
/// ```
/// # Remarks
///
/// The resulting SecretKey is guaranteed to have Jacobi symbol 1.

pub fn create_secnonce(secp: &Secp256k1) -> Result<SecretKey, Error> {
	let nonce = aggsig::export_secnonce_single(secp)?;
	Ok(nonce)
}

/// Calculates a partial signature given the signer's secure key,
/// the sum of all public nonces and (optionally) the sum of all public keys.
///
/// # Arguments
///
/// * `secp` - A Secp256k1 Context initialized for Signing
/// * `sec_key` - The signer's secret key
/// * `sec_nonce` - The signer's secret nonce (the public version of which
/// was added to the `nonce_sum` total)
/// * `nonce_sum` - The sum of the public nonces of all signers participating
/// in the full signature. This value is encoded in e.
/// * `pubkey_sum` - (Optional) The sum of the public keys of all signers participating
/// in the full signature. If included, this value is encoded in e.
/// * `msg` - The message to sign.
///
/// # Example
///
/// ```
/// # extern crate grin_core as core;
/// # extern crate rand;
/// use rand::thread_rng;
/// use core::libtx::aggsig;
/// use util::secp::key::{PublicKey, SecretKey};
/// use util::secp::{ContextFlag, Secp256k1, Message};
///
/// let secp = Secp256k1::with_caps(ContextFlag::SignOnly);
/// let secret_nonce = aggsig::create_secnonce(&secp).unwrap();
/// let secret_key = SecretKey::new(&secp, &mut thread_rng());
/// let pub_nonce_sum = PublicKey::from_secret_key(&secp, &secret_nonce).unwrap();
/// // ... Add all other participating nonces
/// let pub_key_sum = PublicKey::from_secret_key(&secp, &secret_key).unwrap();
/// // ... Add all other participating keys
/// let mut msg_bytes = [0; 32];
/// // ... Encode message
/// let message = Message::from_slice(&msg_bytes).unwrap();
/// let sig_part = aggsig::calculate_partial_sig(
///     &secp,
///     &secret_key,
///     &secret_nonce,
///     &pub_nonce_sum,
///     Some(&pub_key_sum),
///     &message,
///).unwrap();
/// ```

pub fn calculate_partial_sig(
	secp: &Secp256k1,
	sec_key: &SecretKey,
	sec_nonce: &SecretKey,
	nonce_sum: &PublicKey,
	pubkey_sum: Option<&PublicKey>,
	msg: &secp::Message,
) -> Result<Signature, Error> {
	//Now calculate signature using message M=fee, nonce in e=nonce_sum
	let sig = aggsig::sign_single(
		secp,
		&msg,
		sec_key,
		Some(sec_nonce),
		None,
		Some(nonce_sum),
		pubkey_sum,
		Some(nonce_sum),
	)?;
	Ok(sig)
}

/// Verifies a partial signature from a public key. All nonce and public
/// key sum values must be identical to those provided in the call to
/// [`calculate_partial_sig`](fn.calculate_partial_sig.html). Returns
/// `Result::Ok` if the signature is valid, or a Signature
/// [ErrorKind](../enum.ErrorKind.html) otherwise
///
/// # Arguments
///
/// * `secp` - A Secp256k1 Context initialized for Validation
/// * `sig` - The signature to validate, created via a call to
/// [`calculate_partial_sig`](fn.calculate_partial_sig.html)
/// * `pub_nonce_sum` - The sum of the public nonces of all signers participating
/// in the full signature. This value is encoded in e.
/// * `pubkey` - Corresponding Public Key of the private key used to sign the message.
/// * `pubkey_sum` - (Optional) The sum of the public keys of all signers participating
/// in the full signature. If included, this value is encoded in e.
/// * `msg` - The message to verify.
///
/// # Example
///
/// ```
/// # extern crate grin_core as core;
/// # extern crate rand;
/// use rand::thread_rng;
/// use core::libtx::aggsig;
/// use util::secp::key::{PublicKey, SecretKey};
/// use util::secp::{ContextFlag, Secp256k1, Message};
///
/// let secp = Secp256k1::with_caps(ContextFlag::Full);
/// let secret_nonce = aggsig::create_secnonce(&secp).unwrap();
/// let secret_key = SecretKey::new(&secp, &mut thread_rng());
/// let pub_nonce_sum = PublicKey::from_secret_key(&secp, &secret_nonce).unwrap();
/// // ... Add all other participating nonces
/// let pub_key_sum = PublicKey::from_secret_key(&secp, &secret_key).unwrap();
/// // ... Add all other participating keys
/// let mut msg_bytes = [0; 32];
/// // ... Encode message
/// let message = Message::from_slice(&msg_bytes).unwrap();
/// let sig_part = aggsig::calculate_partial_sig(
///     &secp,
///     &secret_key,
///     &secret_nonce,
///     &pub_nonce_sum,
///     Some(&pub_key_sum),
///     &message,
///).unwrap();
///
/// // Now verify the signature, ensuring the same values used to create
/// // the signature are provided:
/// let public_key = PublicKey::from_secret_key(&secp, &secret_key).unwrap();
///
/// let result = aggsig::verify_partial_sig(
///     &secp,
///     &sig_part,
///     &pub_nonce_sum,
///     &public_key,
///     Some(&pub_key_sum),
///     &message,
///);
/// ```

pub fn verify_partial_sig(
	secp: &Secp256k1,
	sig: &Signature,
	pub_nonce_sum: &PublicKey,
	pubkey: &PublicKey,
	pubkey_sum: Option<&PublicKey>,
	msg: &secp::Message,
) -> Result<(), Error> {
	if !verify_single(
		secp,
		sig,
		&msg,
		Some(&pub_nonce_sum),
		pubkey,
		pubkey_sum,
		true,
	) {
		return Err(Error::Signature("Signature validation error".to_string()));
	}
	Ok(())
}

/// Creates a single-signer aggsig signature from a key id. Generally,
/// this function is used to create transaction kernel signatures for
/// coinbase outputs.
/// Returns `Ok(Signature)` if the signature is valid, or a Signature
/// [ErrorKind](../enum.ErrorKind.html) otherwise
///
/// # Arguments
///
/// * `secp` - A Secp256k1 Context initialized for Signing
/// * `k` - The Keychain implementation being used
/// * `msg` - The message to sign (fee|lockheight).
/// * `key_id` - The keychain key id corresponding to the private key
/// with which to sign the message
/// * `blind_sum` - (Optional) The sum of all blinding factors in the transaction
/// in the case of a coinbase transaction this will simply be the corresponding
/// public key.
///
/// # Example
///
/// ```
/// # extern crate grin_core as core;
/// use core::consensus::reward;
/// use util::secp::key::{PublicKey, SecretKey};
/// use util::secp::{ContextFlag, Secp256k1};
/// use core::libtx::{aggsig, proof};
/// use core::core::transaction::KernelFeatures;
/// use core::core::{Output, OutputFeatures};
/// use keychain::{Keychain, ExtKeychain, SwitchCommitmentType};
/// use std::convert::TryInto;
///
/// let secp = Secp256k1::with_caps(ContextFlag::Commit);
/// let keychain = ExtKeychain::from_random_seed(false).unwrap();
/// let fees = 10_000;
/// let value = reward(fees);
/// let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
/// let switch = SwitchCommitmentType::Regular;
/// let commit = keychain.commit(value, &key_id, switch).unwrap();
/// let builder = proof::ProofBuilder::new(&keychain);
/// let proof = proof::create(&keychain, &builder, value, &key_id, switch, commit, None).unwrap();
/// let output = Output::new(OutputFeatures::Coinbase, commit, proof);
/// let height = 20;
/// let over_commit = secp.commit_value(reward(fees)).unwrap();
/// let out_commit = output.commitment();
/// let features = KernelFeatures::HeightLocked{fee: 1.into(), lock_height: height};
/// let msg = features.kernel_sig_msg().unwrap();
/// let excess = secp.commit_sum(vec![out_commit], vec![over_commit]).unwrap();
/// let pubkey = excess.to_pubkey(&secp).unwrap();
/// let sig = aggsig::sign_from_key_id(&secp, &keychain, &msg, value, &key_id, None, Some(&pubkey)).unwrap();
/// ```

pub fn sign_from_key_id<K>(
	secp: &Secp256k1,
	k: &K,
	msg: &Message,
	value: u64,
	key_id: &Identifier,
	s_nonce: Option<&SecretKey>,
	blind_sum: Option<&PublicKey>,
) -> Result<Signature, Error>
where
	K: Keychain,
{
	let skey = k.derive_key(value, key_id, SwitchCommitmentType::Regular)?; // TODO: proper support for different switch commitment schemes
	let sig = aggsig::sign_single(secp, &msg, &skey, s_nonce, None, None, blind_sum, None)?;
	Ok(sig)
}

/// Simple verification a single signature from a commitment. The public
/// key used to verify the signature is derived from the commit.
/// Returns `Ok(())` if the signature is valid, or a Signature
/// [ErrorKind](../enum.ErrorKind.html) otherwise
///
/// # Arguments
///
/// * `secp` - A Secp256k1 Context initialized for Verification
/// * `sig` - The Signature to verify
/// * `msg` - The message to sign (fee|lockheight).
/// * `commit` - The commitment to verify. The actual public key used
/// during verification is derived from this commit.
///
/// # Example
///
/// ```
/// # extern crate grin_core as core;
/// use core::consensus::reward;
/// use core::libtx::{aggsig, proof};
/// use util::secp::key::{PublicKey, SecretKey};
/// use util::secp::{ContextFlag, Secp256k1};
/// use core::core::transaction::KernelFeatures;
/// use core::core::{Output, OutputFeatures};
/// use keychain::{Keychain, ExtKeychain, SwitchCommitmentType};
/// use std::convert::TryInto;
///
/// // Create signature
/// let secp = Secp256k1::with_caps(ContextFlag::Commit);
/// let keychain = ExtKeychain::from_random_seed(false).unwrap();
/// let fees = 10_000;
/// let value = reward(fees);
/// let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
/// let switch = SwitchCommitmentType::Regular;
/// let commit = keychain.commit(value, &key_id, switch).unwrap();
/// let builder = proof::ProofBuilder::new(&keychain);
/// let proof = proof::create(&keychain, &builder, value, &key_id, switch, commit, None).unwrap();
/// let output = Output::new(OutputFeatures::Coinbase, commit, proof);
/// let height = 20;
/// let over_commit = secp.commit_value(reward(fees)).unwrap();
/// let out_commit = output.commitment();
/// let features = KernelFeatures::HeightLocked{fee: 1.into(), lock_height: height};
/// let msg = features.kernel_sig_msg().unwrap();
/// let excess = secp.commit_sum(vec![out_commit], vec![over_commit]).unwrap();
/// let pubkey = excess.to_pubkey(&secp).unwrap();
/// let sig = aggsig::sign_from_key_id(&secp, &keychain, &msg, value, &key_id, None, Some(&pubkey)).unwrap();
///
/// // Verify the signature from the excess commit
/// let sig_verifies =
///     aggsig::verify_single_from_commit(&keychain.secp(), &sig, &msg, &excess);
/// assert!(!sig_verifies.is_err());
/// ```

pub fn verify_single_from_commit(
	secp: &Secp256k1,
	sig: &Signature,
	msg: &Message,
	commit: &Commitment,
) -> Result<(), Error> {
	let pubkey = commit.to_pubkey(secp)?;
	if !verify_single(secp, sig, msg, None, &pubkey, Some(&pubkey), false) {
		return Err(Error::Signature("Signature validation error".to_string()));
	}
	Ok(())
}

/// Verifies a completed (summed) signature, which must include the message
/// and pubkey sum values that are used during signature creation time
/// to create 'e'
/// Returns `Ok(())` if the signature is valid, or a Signature
/// [ErrorKind](../enum.ErrorKind.html) otherwise
///
/// # Arguments
///
/// * `secp` - A Secp256k1 Context initialized for Verification
/// * `sig` - The Signature to verify
/// * `pubkey` - Corresponding Public Key of the private key used to sign the message.
/// * `pubkey_sum` - (Optional) The sum of the public keys of all signers participating
/// in the full signature. If included, this value is encoded in e. Must be the same
/// value as when the signature was created to verify correctly.
/// * `msg` - The message to verify (fee|lockheight).
///
/// # Example
///
/// ```
/// # extern crate grin_core as core;
/// # extern crate rand;
/// use rand::thread_rng;
/// use core::libtx::aggsig;
/// use util::secp::key::{PublicKey, SecretKey};
/// use util::secp::{ContextFlag, Secp256k1, Message};
///
/// let secp = Secp256k1::with_caps(ContextFlag::Full);
/// let secret_nonce = aggsig::create_secnonce(&secp).unwrap();
/// let secret_key = SecretKey::new(&secp, &mut thread_rng());
/// let pub_nonce_sum = PublicKey::from_secret_key(&secp, &secret_nonce).unwrap();
/// // ... Add all other participating nonces
/// let pub_key_sum = PublicKey::from_secret_key(&secp, &secret_key).unwrap();
/// // ... Add all other participating keys
/// let mut msg_bytes = [0; 32];
/// // ... Encode message
/// let message = Message::from_slice(&msg_bytes).unwrap();
/// let sig_part = aggsig::calculate_partial_sig(
///     &secp,
///     &secret_key,
///     &secret_nonce,
///     &pub_nonce_sum,
///     Some(&pub_key_sum),
///     &message,
/// ).unwrap();
/// // ... Verify above, once all signatures have been added together
/// let sig_verifies = aggsig::verify_completed_sig(
///     &secp,
///     &sig_part,
///     &pub_key_sum,
///     Some(&pub_key_sum),
///     &message,
///     );
/// assert!(!sig_verifies.is_err());
/// ```

pub fn verify_completed_sig(
	secp: &Secp256k1,
	sig: &Signature,
	pubkey: &PublicKey,
	pubkey_sum: Option<&PublicKey>,
	msg: &secp::Message,
) -> Result<(), Error> {
	if !verify_single(secp, sig, msg, None, pubkey, pubkey_sum, true) {
		return Err(Error::Signature("Signature validation error".to_string()));
	}
	Ok(())
}

/// Adds signatures
pub fn add_signatures(
	secp: &Secp256k1,
	part_sigs: Vec<&Signature>,
	nonce_sum: &PublicKey,
) -> Result<Signature, Error> {
	// Add public nonces kR*G + kS*G
	let sig = aggsig::add_signatures_single(&secp, part_sigs, &nonce_sum)?;
	Ok(sig)
}

/// Just a simple sig, creates its own nonce if not provided
pub fn sign_single(
	secp: &Secp256k1,
	msg: &Message,
	skey: &SecretKey,
	snonce: Option<&SecretKey>,
	pubkey_sum: Option<&PublicKey>,
) -> Result<Signature, Error> {
	let sig = aggsig::sign_single(secp, &msg, skey, snonce, None, None, pubkey_sum, None)?;
	Ok(sig)
}

/// Verifies an aggsig signature
pub fn verify_single(
	secp: &Secp256k1,
	sig: &Signature,
	msg: &Message,
	pubnonce: Option<&PublicKey>,
	pubkey: &PublicKey,
	pubkey_sum: Option<&PublicKey>,
	is_partial: bool,
) -> bool {
	aggsig::verify_single(
		secp, sig, msg, pubnonce, pubkey, pubkey_sum, None, is_partial,
	)
}

/// Verify a batch of signatures.
pub fn verify_batch(
	secp: &Secp256k1,
	sigs: &Vec<Signature>,
	msgs: &Vec<Message>,
	pubkeys: &Vec<PublicKey>,
) -> bool {
	aggsig::verify_batch(secp, sigs, msgs, pubkeys)
}

/// Just a simple sig, creates its own nonce, etc
pub fn sign_with_blinding(
	secp: &Secp256k1,
	msg: &Message,
	blinding: &BlindingFactor,
	pubkey_sum: Option<&PublicKey>,
) -> Result<Signature, Error> {
	let skey = &blinding.secret_key(&secp)?;
	let sig = aggsig::sign_single(secp, &msg, skey, None, None, None, pubkey_sum, None)?;
	Ok(sig)
}
