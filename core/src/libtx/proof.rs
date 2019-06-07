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
use crate::util::secp::pedersen::{Commitment, ProofMessage, RangeProof};
use crate::util::secp::{self, Secp256k1};
use std::convert::TryFrom;

/// Create a bulletproof
pub fn create<K, B>(
	k: &K,
	b: &B,
	amount: u64,
	key_id: &Identifier,
	switch: &SwitchCommitmentType,
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
	let nonce = b
		.rewind_nonce(&commit)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;
	let info = k
		.secp()
		.rewind_bullet_proof(commit, nonce, extra_data, proof);
	if info.is_err() {
		return Ok(None);
	}
	let info = info.unwrap();

	let amount = info.value;
	let check = b
		.check_output(&commit, amount, info.message)
		.map_err(|e| ErrorKind::RangeProof(e.to_string()))?;

	Ok(check.map(|(id, switch)| (amount, id, switch)))
}

/// Used for building proofs and checking if the output belongs to the wallet
pub trait ProofBuild {
	/// Create a BP nonce that will allow to rewind the derivation path and flags
	fn rewind_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error>;

	/// Create a BP nonce that blinds the private key
	fn private_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error>;

	/// Create a BP message
	fn proof_message(
		&self,
		id: &Identifier,
		switch: &SwitchCommitmentType,
	) -> Result<ProofMessage, Error>;

	/// Check if the output belongs to this keychain
	fn check_output(
		&self,
		commit: &Commitment,
		amount: u64,
		message: ProofMessage,
	) -> Result<Option<(Identifier, SwitchCommitmentType)>, Error>;
}

/// The new, more flexible proof builder
pub struct ProofBuilder<'a, K>
where
	K: Keychain,
{
	keychain: &'a K,
	rewind_hash: Vec<u8>,
	private_hash: Vec<u8>,
}

impl<'a, K> ProofBuilder<'a, K>
where
	K: Keychain,
{
	/// Creates a new instance of this proof builder
	pub fn new(keychain: &'a K) -> Self {
		let mut rewind_root_key = keychain
			.derive_key(0, &K::root_key_id(), &SwitchCommitmentType::None)
			.unwrap()
			.0
			.to_vec();
		let mut private_root_key = rewind_root_key.clone();

		rewind_root_key.push(0);
		private_root_key.push(1);

		let rewind_hash: Vec<u8> = blake2::blake2b::blake2b(32, &[], &rewind_root_key)
			.as_bytes()
			.to_vec();
		let private_hash: Vec<u8> = blake2::blake2b::blake2b(32, &[], &private_root_key)
			.as_bytes()
			.to_vec();

		Self {
			keychain,
			rewind_hash,
			private_hash,
		}
	}

	fn nonce(&self, commit: &Commitment, private: bool) -> Result<SecretKey, Error> {
		let hash = if private {
			&self.private_hash
		} else {
			&self.rewind_hash
		};
		let res = blake2::blake2b::blake2b(32, &commit.0, hash);
		SecretKey::from_slice(self.keychain.secp(), res.as_bytes()).map_err(|e| {
			ErrorKind::RangeProof(format!("Unable to create nonce: {:?}", e).to_string()).into()
		})
	}
}

impl<'a, K> ProofBuild for ProofBuilder<'a, K>
where
	K: Keychain,
{
	fn rewind_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit, false)
	}

	fn private_nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit, true)
	}

	/// Message bytes:
	///   0-1: reserved for future use
	///     2: wallet type (0 for standard)
	///     3: switch commitment type
	///  4-19: derivation path
	fn proof_message(
		&self,
		id: &Identifier,
		switch: &SwitchCommitmentType,
	) -> Result<ProofMessage, Error> {
		let mut msg = [0; 20];
		msg[3] = u8::from(switch);
		let id_ser = id.serialize_path();
		for i in 0..16 {
			msg[i + 4] = id_ser[i];
		}
		Ok(ProofMessage::from_bytes(&msg))
	}

	fn check_output(
		&self,
		commit: &Commitment,
		amount: u64,
		message: ProofMessage,
	) -> Result<Option<(Identifier, SwitchCommitmentType)>, Error> {
		if message.len() != 20 {
			return Ok(None);
		}

		let msg = message.as_bytes();
		let id = Identifier::from_serialized_path(3, &msg[4..]);
		let exp: [u8; 3] = [0; 3];
		if msg[..3] != exp {
			return Ok(None);
		}
		let switch = match SwitchCommitmentType::try_from(msg[3]) {
			Ok(s) => s,
			Err(_) => return Ok(None),
		};

		let commit_exp = self.keychain.commit(amount, &id, &switch)?;
		match commit == &commit_exp {
			true => Ok(Some((id, switch))),
			false => Ok(None),
		}
	}
}

/// The legacy proof builder, used before the first hard fork
pub struct LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	keychain: &'a K,
	root_hash: Vec<u8>,
}

impl<'a, K> LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	/// Creates a new instance of this proof builder
	pub fn new(keychain: &'a K) -> Self {
		Self {
			keychain,
			root_hash: keychain
				.derive_key(0, &K::root_key_id(), &SwitchCommitmentType::Regular)
				.unwrap()
				.0
				.to_vec(),
		}
	}

	fn nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		let res = blake2::blake2b::blake2b(32, &commit.0, &self.root_hash);
		SecretKey::from_slice(self.keychain.secp(), res.as_bytes()).map_err(|e| {
			ErrorKind::RangeProof(format!("Unable to create nonce: {:?}", e).to_string()).into()
		})
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
	fn proof_message(
		&self,
		id: &Identifier,
		_switch: &SwitchCommitmentType,
	) -> Result<ProofMessage, Error> {
		let mut msg = [0; 20];
		let id_ser = id.serialize_path();
		for i in 0..16 {
			msg[i + 4] = id_ser[i];
		}
		Ok(ProofMessage::from_bytes(&msg))
	}

	fn check_output(
		&self,
		commit: &Commitment,
		amount: u64,
		message: ProofMessage,
	) -> Result<Option<(Identifier, SwitchCommitmentType)>, Error> {
		if message.len() != 20 {
			return Ok(None);
		}

		let msg = message.as_bytes();
		let id = Identifier::from_serialized_path(3, &msg[4..]);
		let exp: [u8; 4] = [0; 4];
		if msg[..4] != exp {
			return Ok(None);
		}

		let commit_exp = self
			.keychain
			.commit(amount, &id, &SwitchCommitmentType::Regular)?;
		match commit == &commit_exp {
			true => Ok(Some((id, SwitchCommitmentType::Regular))),
			false => Ok(None),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::keychain::ExtKeychain;
	use rand::{thread_rng, Rng};

	#[test]
	fn legacy_builder() {
		let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();
		let builder = LegacyProofBuilder::new(&keychain);
		let amount = rng.gen();
		let id = ExtKeychain::derive_key_id(3, rng.gen(), rng.gen(), rng.gen(), 0);
		let switch = SwitchCommitmentType::Regular;
		let commit = keychain.commit(amount, &id, &switch).unwrap();
		let proof = create(
			&keychain,
			&builder,
			amount,
			&id,
			&switch,
			commit.clone(),
			None,
		)
		.unwrap();
		assert!(verify(&keychain.secp(), commit.clone(), proof.clone(), None).is_ok());
		let rewind = rewind(&keychain, &builder, commit, None, proof).unwrap();
		assert!(rewind.is_some());
		let (r_amount, r_id, r_switch) = rewind.unwrap();
		assert_eq!(r_amount, amount);
		assert_eq!(r_id, id);
		assert_eq!(r_switch, switch);
	}

	#[test]
	fn builder() {
		let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();
		let builder = ProofBuilder::new(&keychain);
		let amount = rng.gen();
		let id = ExtKeychain::derive_key_id(3, rng.gen(), rng.gen(), rng.gen(), 0);
		// With switch commitment
		let commit_a = {
			let switch = SwitchCommitmentType::Regular;
			let commit = keychain.commit(amount, &id, &switch).unwrap();
			let proof = create(
				&keychain,
				&builder,
				amount,
				&id,
				&switch,
				commit.clone(),
				None,
			)
			.unwrap();
			assert!(verify(&keychain.secp(), commit.clone(), proof.clone(), None).is_ok());
			let rewind = rewind(&keychain, &builder, commit.clone(), None, proof).unwrap();
			assert!(rewind.is_some());
			let (r_amount, r_id, r_switch) = rewind.unwrap();
			assert_eq!(r_amount, amount);
			assert_eq!(r_id, id);
			assert_eq!(r_switch, switch);
			commit
		};
		// Without switch commitment
		let commit_b = {
			let switch = SwitchCommitmentType::None;
			let commit = keychain.commit(amount, &id, &switch).unwrap();
			let proof = create(
				&keychain,
				&builder,
				amount,
				&id,
				&switch,
				commit.clone(),
				None,
			)
			.unwrap();
			assert!(verify(&keychain.secp(), commit.clone(), proof.clone(), None).is_ok());
			let rewind = rewind(&keychain, &builder, commit.clone(), None, proof).unwrap();
			assert!(rewind.is_some());
			let (r_amount, r_id, r_switch) = rewind.unwrap();
			assert_eq!(r_amount, amount);
			assert_eq!(r_id, id);
			assert_eq!(r_switch, switch);
			commit
		};
		// The resulting pedersen commitments should be different
		assert_ne!(commit_a, commit_b);
	}
}
