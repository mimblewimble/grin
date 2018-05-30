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

//! Rangeproof library functions

use blake2;
use keychain::Keychain;
use keychain::extkey::Identifier;
use libtx::error::Error;
use util::logger::LOGGER;
use util::secp::key::SecretKey;
use util::secp::pedersen::{Commitment, ProofInfo, ProofMessage, RangeProof};
use util::secp::{self, Secp256k1};

fn create_nonce(k: &Keychain, commit: &Commitment) -> Result<SecretKey, Error> {
	// hash(commit|masterkey) as nonce
	let root_key = k.root_key_id().to_bytes();
	let res = blake2::blake2b::blake2b(32, &commit.0, &root_key);
	let res = res.as_bytes();
	let mut ret_val = [0; 32];
	for i in 0..res.len() {
		ret_val[i] = res[i];
	}
	match SecretKey::from_slice(k.secp(), &ret_val) {
		Ok(sk) => Ok(sk),
		Err(e) => Err(Error::RangeProof(
			format!("Unable to create nonce: {:?}", e).to_string(),
		)),
	}
}

/// So we want this to take an opaque structure that can be called
/// back to get the sensitive data

pub fn create(
	k: &Keychain,
	amount: u64,
	key_id: &Identifier,
	_commit: Commitment,
	extra_data: Option<Vec<u8>>,
	msg: ProofMessage,
) -> Result<RangeProof, Error> {
	let commit = k.commit(amount, key_id)?;
	let skey = k.derived_key(key_id)?;
	let nonce = create_nonce(k, &commit)?;
	if msg.len() == 0 {
		return Ok(k.secp().bullet_proof(amount, skey, nonce, extra_data, None));
	} else {
		if msg.len() != 64 {
			error!(LOGGER, "Bullet proof message must be 64 bytes.");
			return Err(Error::RangeProof(
				"Bullet proof message must be 64 bytes".to_string(),
			));
		}
	}
	return Ok(k.secp()
		.bullet_proof(amount, skey, nonce, extra_data, Some(msg)));
}

/// Verify a proof
pub fn verify(
	secp: &Secp256k1,
	commit: Commitment,
	proof: RangeProof,
	extra_data: Option<Vec<u8>>,
) -> Result<(), secp::Error> {
	let result = secp.verify_bullet_proof(commit, proof, extra_data);
	match result {
		Ok(_) => Ok(()),
		Err(e) => Err(e),
	}
}

/// Rewind a rangeproof to retrieve the amount
pub fn rewind(
	k: &Keychain,
	key_id: &Identifier,
	commit: Commitment,
	extra_data: Option<Vec<u8>>,
	proof: RangeProof,
) -> Result<ProofInfo, Error> {
	let skey = k.derived_key(key_id)?;
	let nonce = create_nonce(k, &commit)?;
	let proof_message = k.secp()
		.unwind_bullet_proof(commit, skey, nonce, extra_data, proof);
	let proof_info = match proof_message {
		Ok(p) => ProofInfo {
			success: true,
			value: 0,
			message: p,
			mlen: 0,
			min: 0,
			max: 0,
			exp: 0,
			mantissa: 0,
		},
		Err(_) => ProofInfo {
			success: false,
			value: 0,
			message: ProofMessage::empty(),
			mlen: 0,
			min: 0,
			max: 0,
			exp: 0,
			mantissa: 0,
		},
	};
	return Ok(proof_info);
}
