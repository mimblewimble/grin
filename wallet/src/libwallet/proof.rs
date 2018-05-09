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

use keychain::Keychain;
use util::secp::pedersen::{Commitment, ProofInfo, ProofMessage, RangeProof};
use util::secp::key::SecretKey;
use util::secp::{self, Secp256k1};
use keychain::extkey::Identifier;
use libwallet::error::Error;
use blake2;

pub fn create_nonce(k: &Keychain, commit: &Commitment) -> SecretKey {
	// hash(commit|masterkey) as nonce
	let root_key = k.root_key_id().to_bytes();
	let res = blake2::blake2b::blake2b(32, &commit.0, &root_key);
	let res = res.as_bytes();
	let mut ret_val = [0; 32];
	for i in 0..res.len() {
		ret_val[i] = res[i];
	}
	SecretKey::from_slice(k.secp(), &ret_val).unwrap()
}

/// So we want this to take an opaque structure that can be called
/// back to get the sensitive data

pub fn create(
	k: &Keychain,
	amount: u64,
	key_id: &Identifier,
	_commit: Commitment,
	extra_data: Option<Vec<u8>>,
) -> Result<RangeProof, Error> {
	let commit = k.commit(amount, key_id)?;
	let skey = k.derived_key(key_id)?;
	let nonce = create_nonce(k, &commit);
	Ok(k.secp().bullet_proof(amount, skey, nonce, extra_data))
}

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

pub fn rewind(
	k: &Keychain,
	commit: Commitment,
	extra_data: Option<Vec<u8>>,
	proof: RangeProof,
) -> Result<ProofInfo, Error> {
	let nonce = create_nonce(k, &commit);
	let proof_message = k.secp()
		.rewind_bullet_proof(commit, nonce, extra_data, proof);
	let proof_info = match proof_message {
		Ok(p) => p,
		Err(_) => ProofInfo {
			success: false,
			value: 0,
			message: ProofMessage::empty(),
			blinding: SecretKey([0; secp::constants::SECRET_KEY_SIZE]),
			mlen: 0,
			min: 0,
			max: 0,
			exp: 0,
			mantissa: 0,
		},
	};
	return Ok(proof_info);
}
