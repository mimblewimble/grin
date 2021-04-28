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

//! Rangeproof library functions

use crate::libtx::error::Error;
use blake2::blake2b::blake2b;
use keychain::extkey_bip32::BIP32GrinHasher;
use keychain::{Identifier, Keychain, SwitchCommitmentType, ViewKey};
use std::convert::TryFrom;
use util::secp::key::SecretKey;
use util::secp::pedersen::{Commitment, ProofMessage, RangeProof};
use util::secp::{self, Secp256k1};
use zeroize::Zeroize;

/// Create a bulletproof
pub fn create<K, B>(
	k: &K,
	b: &B,
	amount: u64,
	key_id: &Identifier,
	switch: SwitchCommitmentType,
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
	let secp = k.secp();
	let commit = k.commit(amount, key_id, switch)?;
	let skey = k.derive_key(amount, key_id, switch)?;
	let rewind_nonce = b.rewind_nonce(secp, &commit)?;
	let private_nonce = b.private_nonce(secp, &commit)?;
	let message = b.proof_message(secp, key_id, switch)?;
	Ok(secp.bullet_proof(
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
	result.map(|_| ())
}

/// Rewind a rangeproof to retrieve the amount, derivation path and switch commitment type
pub fn rewind<B>(
	secp: &Secp256k1,
	b: &B,
	commit: Commitment,
	extra_data: Option<Vec<u8>>,
	proof: RangeProof,
) -> Result<Option<(u64, Identifier, SwitchCommitmentType)>, Error>
where
	B: ProofBuild,
{
	let nonce = b
		.rewind_nonce(secp, &commit)
		.map_err(|e| Error::RangeProof(e.to_string()))?;
	let info = secp.rewind_bullet_proof(commit, nonce, extra_data, proof);
	if info.is_err() {
		return Ok(None);
	}
	let info = info.unwrap();

	let amount = info.value;
	let check = b
		.check_output(secp, &commit, amount, info.message)
		.map_err(|e| Error::RangeProof(e.to_string()))?;

	Ok(check.map(|(id, switch)| (amount, id, switch)))
}

/// Used for building proofs and checking if the output belongs to the wallet
pub trait ProofBuild {
	/// Create a BP nonce that will allow to rewind the derivation path and flags
	fn rewind_nonce(&self, secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error>;

	/// Create a BP nonce that blinds the private key
	fn private_nonce(&self, secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error>;

	/// Create a BP message
	fn proof_message(
		&self,
		secp: &Secp256k1,
		id: &Identifier,
		switch: SwitchCommitmentType,
	) -> Result<ProofMessage, Error>;

	/// Check if the output belongs to this keychain
	fn check_output(
		&self,
		secp: &Secp256k1,
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
		let private_root_key = keychain
			.derive_key(0, &K::root_key_id(), SwitchCommitmentType::None)
			.unwrap();

		let private_hash = blake2b(32, &[], &private_root_key.0).as_bytes().to_vec();

		let public_root_key = keychain
			.public_root_key()
			.serialize_vec(keychain.secp(), true);
		let rewind_hash = blake2b(32, &[], &public_root_key[..]).as_bytes().to_vec();

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
		let res = blake2b(32, &commit.0, hash);
		SecretKey::from_slice(self.keychain.secp(), res.as_bytes())
			.map_err(|e| Error::RangeProof(format!("Unable to create nonce: {:?}", e)))
	}
}

impl<'a, K> ProofBuild for ProofBuilder<'a, K>
where
	K: Keychain,
{
	fn rewind_nonce(&self, _secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit, false)
	}

	fn private_nonce(&self, _secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit, true)
	}

	/// Message bytes:
	///     0: reserved for future use
	///     1: wallet type (0 for standard)
	///     2: switch commitment type
	///     3: path depth
	///  4-19: derivation path
	fn proof_message(
		&self,
		_secp: &Secp256k1,
		id: &Identifier,
		switch: SwitchCommitmentType,
	) -> Result<ProofMessage, Error> {
		let mut msg = [0; 20];
		msg[2] = switch as u8;
		let id_bytes = id.to_bytes();
		msg[3..20].clone_from_slice(&id_bytes[..17]);
		Ok(ProofMessage::from_bytes(&msg))
	}

	fn check_output(
		&self,
		_secp: &Secp256k1,
		commit: &Commitment,
		amount: u64,
		message: ProofMessage,
	) -> Result<Option<(Identifier, SwitchCommitmentType)>, Error> {
		if message.len() != 20 {
			return Ok(None);
		}
		let msg = message.as_bytes();
		let exp: [u8; 2] = [0; 2];
		if msg[..2] != exp {
			return Ok(None);
		}
		let switch = match SwitchCommitmentType::try_from(msg[2]) {
			Ok(s) => s,
			Err(_) => return Ok(None),
		};
		let depth = u8::min(msg[3], 4);
		let id = Identifier::from_serialized_path(depth, &msg[4..]);

		let commit_exp = self.keychain.commit(amount, &id, switch)?;
		if commit == &commit_exp {
			Ok(Some((id, switch)))
		} else {
			Ok(None)
		}
	}
}

impl<'a, K> Zeroize for ProofBuilder<'a, K>
where
	K: Keychain,
{
	fn zeroize(&mut self) {
		self.rewind_hash.zeroize();
		self.private_hash.zeroize();
	}
}

impl<'a, K> Drop for ProofBuilder<'a, K>
where
	K: Keychain,
{
	fn drop(&mut self) {
		self.zeroize();
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
				.derive_key(0, &K::root_key_id(), SwitchCommitmentType::Regular)
				.unwrap()
				.0
				.to_vec(),
		}
	}

	fn nonce(&self, commit: &Commitment) -> Result<SecretKey, Error> {
		let res = blake2b(32, &commit.0, &self.root_hash);
		SecretKey::from_slice(self.keychain.secp(), res.as_bytes())
			.map_err(|e| Error::RangeProof(format!("Unable to create nonce: {:?}", e)))
	}
}

impl<'a, K> ProofBuild for LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	fn rewind_nonce(&self, _secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit)
	}

	fn private_nonce(&self, _secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error> {
		self.nonce(commit)
	}

	/// Message bytes:
	///   0-3: 0
	///  4-19: derivation path
	/// All outputs with this scheme are assumed to use regular switch commitments
	fn proof_message(
		&self,
		_secp: &Secp256k1,
		id: &Identifier,
		_switch: SwitchCommitmentType,
	) -> Result<ProofMessage, Error> {
		let mut msg = [0; 20];
		let id_ser = id.serialize_path();
		msg[4..20].clone_from_slice(&id_ser[..16]);
		Ok(ProofMessage::from_bytes(&msg))
	}

	fn check_output(
		&self,
		_secp: &Secp256k1,
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
			.commit(amount, &id, SwitchCommitmentType::Regular)?;
		if commit == &commit_exp {
			Ok(Some((id, SwitchCommitmentType::Regular)))
		} else {
			Ok(None)
		}
	}
}

impl<'a, K> Zeroize for LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	fn zeroize(&mut self) {
		self.root_hash.zeroize();
	}
}

impl<'a, K> Drop for LegacyProofBuilder<'a, K>
where
	K: Keychain,
{
	fn drop(&mut self) {
		self.zeroize();
	}
}

impl ProofBuild for ViewKey {
	fn rewind_nonce(&self, secp: &Secp256k1, commit: &Commitment) -> Result<SecretKey, Error> {
		let res = blake2b(32, &commit.0, &self.rewind_hash);
		SecretKey::from_slice(secp, res.as_bytes())
			.map_err(|e| Error::RangeProof(format!("Unable to create nonce: {:?}", e)))
	}

	fn private_nonce(&self, _secp: &Secp256k1, _commit: &Commitment) -> Result<SecretKey, Error> {
		unimplemented!();
	}

	fn proof_message(
		&self,
		_secp: &Secp256k1,
		_id: &Identifier,
		_switch: SwitchCommitmentType,
	) -> Result<ProofMessage, Error> {
		unimplemented!();
	}

	fn check_output(
		&self,
		secp: &Secp256k1,
		commit: &Commitment,
		amount: u64,
		message: ProofMessage,
	) -> Result<Option<(Identifier, SwitchCommitmentType)>, Error> {
		if message.len() != 20 {
			return Ok(None);
		}
		let msg = message.as_bytes();
		let exp: [u8; 2] = [0; 2];
		if msg[..2] != exp {
			return Ok(None);
		}
		let switch = match SwitchCommitmentType::try_from(msg[2]) {
			Ok(s) => s,
			Err(_) => return Ok(None),
		};
		let depth = u8::min(msg[3], 4);
		let id = Identifier::from_serialized_path(depth, &msg[4..]);

		let path = id.to_path();
		if self.depth > path.depth {
			return Ok(None);
		}

		// For non-root key, check child number of current depth
		if self.depth > 0
			&& path.depth > 0
			&& self.child_number != path.path[self.depth as usize - 1]
		{
			return Ok(None);
		}

		let mut key = self.clone();
		let mut hasher = BIP32GrinHasher::new(self.is_test);
		for i in self.depth..path.depth {
			let child_number = path.path[i as usize];
			if child_number.is_hardened() {
				return Ok(None);
			}
			key = key.ckd_pub(&secp, &mut hasher, child_number)?;
		}
		let pub_key = key.commit(secp, amount, switch)?;
		if commit.to_pubkey(&secp)? == pub_key {
			Ok(Some((id, switch)))
		} else {
			Ok(None)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use keychain::ChildNumber;
	use keychain::ExtKeychain;
	use rand::{thread_rng, Rng};

	#[test]
	fn legacy_builder() {
		let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();
		let builder = LegacyProofBuilder::new(&keychain);
		let amount = rng.gen();
		let id = ExtKeychain::derive_key_id(3, rng.gen(), rng.gen(), rng.gen(), 0);
		let switch = SwitchCommitmentType::Regular;
		let commit = keychain.commit(amount, &id, switch).unwrap();
		let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
		assert!(verify(&keychain.secp(), commit, proof, None).is_ok());
		let rewind = rewind(keychain.secp(), &builder, commit, None, proof).unwrap();
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
			let commit = keychain.commit(amount, &id, switch).unwrap();
			let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
			assert!(verify(&keychain.secp(), commit, proof, None).is_ok());
			let rewind = rewind(keychain.secp(), &builder, commit, None, proof).unwrap();
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
			let commit = keychain.commit(amount, &id, switch).unwrap();
			let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
			assert!(verify(&keychain.secp(), commit, proof, None).is_ok());
			let rewind = rewind(keychain.secp(), &builder, commit, None, proof).unwrap();
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

	#[test]
	fn view_key() {
		// TODO
		/*let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();

		let builder = ProofBuilder::new(&keychain);
		let mut hasher = keychain.hasher();
		let view_key = ViewKey::create(&keychain, keychain.master.clone(), &mut hasher, false).unwrap();
		assert_eq!(builder.rewind_hash, view_key.rewind_hash);

		let amount = rng.gen();
		//let id = ExtKeychain::derive_key_id(3, rng.gen::<u16>() as u32, rng.gen::<u16>() as u32, rng.gen::<u16>() as u32, 0);
		let id = ExtKeychain::derive_key_id(0, 0, 0, 0, 0);
		let switch = SwitchCommitmentType::Regular;
		println!("commit_0 = {:?}", keychain.commit(amount, &id, SwitchCommitmentType::None).unwrap().0.to_vec());
		let commit = keychain.commit(amount, &id, &switch).unwrap();

		// Generate proof with ProofBuilder..
		let proof = create(&keychain, &builder, amount, &id, &switch, commit.clone(), None).unwrap();
		// ..and rewind with ViewKey
		let rewind = rewind(keychain.secp(), &view_key, commit.clone(), None, proof);

		assert!(rewind.is_ok());
		let rewind = rewind.unwrap();
		assert!(rewind.is_some());
		let (r_amount, r_id, r_switch) = rewind.unwrap();
		assert_eq!(r_amount, amount);
		assert_eq!(r_id, id);
		assert_eq!(r_switch, switch);*/
	}

	#[test]
	fn view_key_no_switch() {
		let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();

		let builder = ProofBuilder::new(&keychain);
		let mut hasher = keychain.hasher();
		let view_key =
			ViewKey::create(&keychain, keychain.master.clone(), &mut hasher, false).unwrap();
		assert_eq!(builder.rewind_hash, view_key.rewind_hash);

		let amount = rng.gen();
		let id = ExtKeychain::derive_key_id(
			3,
			rng.gen::<u16>() as u32,
			rng.gen::<u16>() as u32,
			rng.gen::<u16>() as u32,
			0,
		);
		let switch = SwitchCommitmentType::None;
		let commit = keychain.commit(amount, &id, switch).unwrap();

		// Generate proof with ProofBuilder..
		let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
		// ..and rewind with ViewKey
		let rewind = rewind(keychain.secp(), &view_key, commit, None, proof);

		assert!(rewind.is_ok());
		let rewind = rewind.unwrap();
		assert!(rewind.is_some());
		let (r_amount, r_id, r_switch) = rewind.unwrap();
		assert_eq!(r_amount, amount);
		assert_eq!(r_id, id);
		assert_eq!(r_switch, switch);
	}

	#[test]
	fn view_key_hardened() {
		let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();

		let builder = ProofBuilder::new(&keychain);
		let mut hasher = keychain.hasher();
		let view_key =
			ViewKey::create(&keychain, keychain.master.clone(), &mut hasher, false).unwrap();
		assert_eq!(builder.rewind_hash, view_key.rewind_hash);

		let amount = rng.gen();
		let id = ExtKeychain::derive_key_id(
			3,
			rng.gen::<u16>() as u32,
			u32::max_value() - 2,
			rng.gen::<u16>() as u32,
			0,
		);
		let switch = SwitchCommitmentType::None;
		let commit = keychain.commit(amount, &id, switch).unwrap();

		// Generate proof with ProofBuilder..
		let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
		// ..and rewind with ViewKey
		let rewind = rewind(keychain.secp(), &view_key, commit, None, proof);

		assert!(rewind.is_ok());
		let rewind = rewind.unwrap();
		assert!(rewind.is_none());
	}

	#[test]
	fn view_key_child() {
		let rng = &mut thread_rng();
		let keychain = ExtKeychain::from_random_seed(false).unwrap();

		let builder = ProofBuilder::new(&keychain);
		let mut hasher = keychain.hasher();
		let view_key =
			ViewKey::create(&keychain, keychain.master.clone(), &mut hasher, false).unwrap();
		assert_eq!(builder.rewind_hash, view_key.rewind_hash);

		// Same child
		{
			let child_view_key = view_key
				.ckd_pub(
					keychain.secp(),
					&mut hasher,
					ChildNumber::from_normal_idx(10),
				)
				.unwrap();
			assert_eq!(child_view_key.depth, 1);

			let amount = rng.gen();
			let id = ExtKeychain::derive_key_id(
				3,
				10,
				rng.gen::<u16>() as u32,
				rng.gen::<u16>() as u32,
				0,
			);
			let switch = SwitchCommitmentType::None;
			let commit = keychain.commit(amount, &id, switch).unwrap();

			// Generate proof with ProofBuilder..
			let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
			// ..and rewind with child ViewKey
			let rewind = rewind(keychain.secp(), &child_view_key, commit, None, proof);

			assert!(rewind.is_ok());
			let rewind = rewind.unwrap();
			assert!(rewind.is_some());
			let (r_amount, r_id, r_switch) = rewind.unwrap();
			assert_eq!(r_amount, amount);
			assert_eq!(r_id, id);
			assert_eq!(r_switch, switch);
		}

		// Different child
		{
			let child_view_key = view_key
				.ckd_pub(
					keychain.secp(),
					&mut hasher,
					ChildNumber::from_normal_idx(11),
				)
				.unwrap();
			assert_eq!(child_view_key.depth, 1);

			let amount = rng.gen();
			let id = ExtKeychain::derive_key_id(
				3,
				10,
				rng.gen::<u16>() as u32,
				rng.gen::<u16>() as u32,
				0,
			);
			let switch = SwitchCommitmentType::None;
			let commit = keychain.commit(amount, &id, switch).unwrap();

			// Generate proof with ProofBuilder..
			let proof = create(&keychain, &builder, amount, &id, switch, commit, None).unwrap();
			// ..and rewind with child ViewKey
			let rewind = rewind(keychain.secp(), &child_view_key, commit, None, proof);

			assert!(rewind.is_ok());
			let rewind = rewind.unwrap();
			assert!(rewind.is_none());
		}
	}
}
