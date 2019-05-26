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

use crate::keychain::{Identifier, Keychain};
use crate::libtx::error::{Error, ErrorKind};
use crate::util::secp::key::SecretKey;
use crate::util::secp::pedersen::{Commitment, ProofInfo, ProofMessage, RangeProof};
use crate::util::secp::{self, Secp256k1};

/// Create a bulletproof
pub fn create<K>(
	k: &K,
	amount: u64,
	key_id: &Identifier,
	_commit: Commitment,
	extra_data: Option<Vec<u8>>,
) -> Result<RangeProof, Error>
where
	K: Keychain,
{
	let commit = k.commit(amount, key_id)?;
	let skey = k.derive_key(amount, key_id)?;
	let legacy = true; // TODO: set to false when ready
	let rewind_nonce = k
		.create_rewind_nonce(&commit, legacy)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;
	let private_nonce = k
		.create_private_nonce(&commit, legacy)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;
	let message = k.create_proof_message(key_id, legacy);
	Ok(k.secp()
		.bullet_proof(amount, skey, rewind_nonce, private_nonce, extra_data, Some(message)))
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
pub fn rewind<K>(
	k: &K,
	commit: Commitment,
	extra_data: Option<Vec<u8>>,
	legacy: bool,
	proof: RangeProof,
) -> Result<ProofInfo, Error>
where
	K: Keychain,
{
	let nonce = k
		.create_rewind_nonce(&commit, legacy)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;
	let proof_message = k
		.secp()
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
