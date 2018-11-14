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
//! Aggsig helper functions used in transaction creation.. should be only
//! interface into the underlying secp library

use keychain::{BlindingFactor, Identifier, Keychain};
use libtx::error::{Error, ErrorKind};
use util::secp::key::{PublicKey, SecretKey};
use util::secp::pedersen::Commitment;
use util::secp::{self, aggsig, Message, Secp256k1, Signature};

/// exports a secure nonce guaranteed to be usable
/// in aggsig creation
pub fn create_secnonce(secp: &Secp256k1) -> Result<SecretKey, Error> {
	let nonce = aggsig::export_secnonce_single(secp)?;
	Ok(nonce)
}

/// Calculate a partial sig
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

/// Verifies a partial sig given all public nonces used in the round
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
		Err(ErrorKind::Signature(
			"Signature validation error".to_string(),
		))?
	}
	Ok(())
}

/// Just a simple sig, creates its own nonce, etc
pub fn sign_from_key_id<K>(
	secp: &Secp256k1,
	k: &K,
	msg: &Message,
	key_id: &Identifier,
	blind_sum: Option<&PublicKey>,
) -> Result<Signature, Error>
where
	K: Keychain,
{
	let skey = k.derive_key(key_id)?;
	let sig = aggsig::sign_single(
		secp,
		&msg,
		&skey.secret_key,
		None,
		None,
		None,
		blind_sum,
		None,
	)?;
	Ok(sig)
}

/// Verifies a sig given a commitment
pub fn verify_single_from_commit(
	secp: &Secp256k1,
	sig: &Signature,
	msg: &Message,
	commit: &Commitment,
) -> Result<(), Error> {
	let pubkey = commit.to_pubkey(secp)?;
	if !verify_single(secp, sig, msg, None, &pubkey, Some(&pubkey), false) {
		Err(ErrorKind::Signature(
			"Signature validation error".to_string(),
		))?
	}
	Ok(())
}

/// Verify a sig, with built message
pub fn verify_sig_build_msg(
	secp: &Secp256k1,
	sig: &Signature,
	pubkey: &PublicKey,
	pubkey_sum: Option<&PublicKey>,
	msg: &secp::Message,
) -> Result<(), Error> {
	if !verify_single(secp, sig, msg, None, pubkey, pubkey_sum, true) {
		Err(ErrorKind::Signature(
			"Signature validation error".to_string(),
		))?
	}
	Ok(())
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

/// Just a simple sig, creates its own nonce, etc
pub fn sign_with_blinding(
	secp: &Secp256k1,
	msg: &Message,
	blinding: &BlindingFactor,
	pubkey_sum: Option<&PublicKey>,
) -> Result<Signature, Error> {
	let skey = &blinding.secret_key(&secp)?;
	//let pubkey_sum = PublicKey::from_secret_key(&secp, &skey)?;
	let sig = aggsig::sign_single(secp, &msg, skey, None, None, None, pubkey_sum, None)?;
	Ok(sig)
}
