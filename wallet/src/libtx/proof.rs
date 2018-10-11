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
use keychain::{Identifier, Keychain};
use libtx::error::{Error, ErrorKind};
use util::secp::key::SecretKey;
use util::secp::pedersen::{Commitment, ProofInfo, ProofMessage, RangeProof};
use util::secp::{self, Secp256k1};

fn create_nonce<K>(k: &K, commit: &Commitment) -> Result<SecretKey, Error>
where
	K: Keychain,
{
	// hash(commit|wallet root secret key (m)) as nonce
	let root_key = k.derive_key(&K::root_key_id())?.secret_key;
	let res = blake2::blake2b::blake2b(32, &commit.0, &root_key.0[..]);
	let res = res.as_bytes();
	let mut ret_val = [0; 32];
	for i in 0..res.len() {
		ret_val[i] = res[i];
	}
	match SecretKey::from_slice(k.secp(), &ret_val) {
		Ok(sk) => Ok(sk),
		Err(e) => Err(ErrorKind::RangeProof(
			format!("Unable to create nonce: {:?}", e).to_string(),
		))?,
	}
}

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
	let skey = k.derive_key(key_id)?;
	let nonce = create_nonce(k, &commit)?;
	let message = ProofMessage::from_bytes(&key_id.serialize_path());
	Ok(k.secp()
		.bullet_proof(amount, skey.secret_key, nonce, extra_data, Some(message)))
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
	proof: RangeProof,
) -> Result<ProofInfo, Error>
where
	K: Keychain,
{
	let nonce = create_nonce(k, &commit)?;
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
