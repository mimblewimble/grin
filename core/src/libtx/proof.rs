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

use crate::blake2;
use crate::keychain::{Identifier, Keychain, SwitchCommitmentType};
use crate::libtx::error::{Error, ErrorKind};
use crate::util::secp::key::SecretKey;
use crate::util::secp::pedersen::{Commitment, ProofInfo, ProofMessage, RangeProof};
use crate::util::secp::{self, Secp256k1};

/// Create a bulletproof
pub fn create<K, B>(
	k: &K,
	b: &B,
	amount: u64,
	key_id: &Identifier,
	_commit: Commitment,
	extra_data: Option<Vec<u8>>,
) -> Result<RangeProof, Error>
where
	K: Keychain,
	B: ProofBuild,
{
	// TODO: proper support for different switch commitment schemes
	// The new bulletproof scheme encodes and decodes it, but
	// it is not supported at the wallet level (yet).
	// So for now we only build outputs with switch commitments
	let switch = &SwitchCommitmentType::Regular;
	let commit = k.commit(amount, key_id, switch)?;
	let skey = k.derive_key(amount, key_id, switch)?;
	let rewind_nonce = b.rewind_nonce(&commit)?;
	let private_nonce = b.private_nonce(&commit)?;
	let message = b.proof_message(key_id, switch)?;
	Ok(k.secp().bullet_proof(
		amount,
		skey,
		rewind_nonce,
		private_nonce,
		extra_data,
		Some(message),
	))
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

/// Rewind a rangeproof to retrieve the amount, derivation path and switch commitment type
pub fn rewind<K, B>(
	k: &K,
	b: &B,
	commit: Commitment,
	extra_data: Option<Vec<u8>>,
	proof: RangeProof,
) -> Result<Option<(u64, Identifier, SwitchCommitmentType)>, Error>
where
	K: Keychain,
	B: ProofBuild,
{
	let nonce = b.rewind_nonce(&commit)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;
	let info = k
		.secp()
		.rewind_bullet_proof(commit, nonce, extra_data, proof)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;

	let amount = info.value;
	let check = b.check_output(&commit, amount, info.message)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;

	Ok(check.map(|(id, switch)| (amount, id, switch)))
}

pub trait ProofBuild {
	/// Create a BP nonce that will allow to rewind the derivation path and flags
	fn rewind_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error>;

	/// Create a BP nonce that blinds the private key
	fn private_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error>;

	/// Create a BP message
	fn proof_message(&self, id: &Identifier, switch: &SwitchCommitmentType) -> Result<ProofMessage, Error>;

	/// Check if the output belongs to this keychain
	fn check_output(&self, commit: &Commitment, amount: u64, message: ProofMessage)
		-> Result<Option<(Identifier, SwitchCommitmentType)>, Error>;
}

pub struct ProofBuilder<'a, K>
where
	K: Keychain,
{
	keychain: &'a K,
}

impl<'a, K> ProofBuilder<'a, K>
where
	K: Keychain,
{
	/// Creates a new instance of this proof builder
	pub fn new(keychain: &'a K) -> Self {
		Self {
			keychain,
		}
	}

	fn nonce(&self, commit: &Commitment, extra_data: u8) -> Result<SecretKey, Error> {
		let mut root_key = self.keychain.derive_key(0, &K::root_key_id(), &SwitchCommitmentType::None)?.0.to_vec();
		root_key.push(extra_data);
		let root_key_hash = blake2::blake2b::blake2b(32, &[], &root_key);
		let res = blake2::blake2b::blake2b(32, &commit.0, root_key_hash.as_bytes());
		SecretKey::from_slice(self.keychain.secp(), res.as_bytes())
			.map_err(|e| ErrorKind::RangeProof(format!("Unable to create nonce: {:?}", e).to_string()).into())
	}
}

impl<'a, K> ProofBuild for ProofBuilder<'a, K>
where
	K: Keychain,
{
	fn rewind_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit, 0)
	}

	fn private_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit, 1)
	}

	/// Message bytes:
	///   0-1: reserved for future use
	///     2: wallet type (0 for standard)
	///     3: switch commitment type (0 for none, 1 for standard)
	///  4-19: derivation path
	fn proof_message(&self, id: &Identifier, switch: &SwitchCommitmentType) -> Result<ProofMessage, Error> {
		let mut msg = [0; 20];
		msg[3] = match *switch {
			SwitchCommitmentType::Regular => 1,
			_ => 0,
		};
		let id_ser = id.serialize_path();
		for i in 0..16 {
			msg[i + 4] = id_ser[i];
		}
		Ok(ProofMessage::from_bytes(&msg))
	}

	fn check_output(&self, commit: &Commitment, amount: u64, message: ProofMessage)
		-> Result<Option<(Identifier, SwitchCommitmentType)>, Error> {
		if message.len() != 20 {
			return Ok(None);
		}

		let msg = message.as_bytes();
		let id = Identifier::from_serialized_path(3, &msg[4..]);
		let exp: [u8; 3] = [0; 3];
		if msg[..3] != exp {
			return Ok(None);
		}
		let switch = match msg[3] {
			1 => SwitchCommitmentType::Regular,
			_ => SwitchCommitmentType::None,
		};

		let commit_exp = self.keychain.commit(amount, &id, &switch)?;
		match commit == &commit_exp {
			true => Ok(Some((id, switch))),
			false => Ok(None),
		}
	}
}

pub struct LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	keychain: &'a K,
}

impl<'a, K> LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	/// Creates a new instance of this proof builder
	pub fn new(keychain: &'a K) -> Self {
		Self {
			keychain,
		}
	}

	fn nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		let root_key = self.keychain.derive_key(0, &K::root_key_id(), &SwitchCommitmentType::Regular)?;
		let res = blake2::blake2b::blake2b(32, &commit.0, &root_key.0[..]);
		SecretKey::from_slice(self.keychain.secp(), res.as_bytes())
			.map_err(|e| ErrorKind::RangeProof(format!("Unable to create nonce: {:?}", e).to_string()).into())
	}
}

impl<'a, K> ProofBuild for LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	fn rewind_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit)
	}

	fn private_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit)
	}

	/// Message bytes:
	///   0-3: 0
	///  4-19: derivation path
	/// All outputs with this scheme are assumed to use regular switch commitments
	fn proof_message(&self, id: &Identifier, _switch: &SwitchCommitmentType) -> Result<ProofMessage, Error> {
		let mut msg = [0; 20];
		let id_ser = id.serialize_path();
		for i in 0..16 {
			msg[i + 4] = id_ser[i];
		}
		Ok(ProofMessage::from_bytes(&msg))
	}

	fn check_output(&self, commit: &Commitment, amount: u64, message: ProofMessage)
		-> Result<Option<(Identifier, SwitchCommitmentType)>, Error> {
		if message.len() != 20 {
			return Ok(None);
		}

		let msg = message.as_bytes();
		let id = Identifier::from_serialized_path(3, &msg[4..]);
		let exp: [u8; 4] = [0; 4];
		if msg[..4] != exp {
			return Ok(None);
		}

		let commit_exp = self.keychain.commit(amount, &id, &SwitchCommitmentType::Regular)?;
		match commit == &commit_exp {
			true => Ok(Some((id, SwitchCommitmentType::Regular))),
			false => Ok(None),
		}
	}
}
