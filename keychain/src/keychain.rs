// Copyright 2017 The Grin Developers
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

use rand::{thread_rng, Rng};

use secp;
use secp::{Message, Secp256k1, Signature};
use secp::key::SecretKey;
use secp::pedersen::{Commitment, RangeProof};
use blake2;

use extkey::{self, Fingerprint, Identifier};


#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Error {
	ExtendedKey(extkey::Error),
	Secp(secp::Error),
	KeyDerivation(String),
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error { Error::Secp(e) }
}

impl From<extkey::Error> for Error {
	fn from(e: extkey::Error) -> Error { Error::ExtendedKey(e) }
}

/// Encapsulate a secret key for the blind_sum operation
#[derive(Clone, Debug)]
pub struct BlindingFactor(secp::key::SecretKey);

impl BlindingFactor {
	fn secret_key(&self) -> secp::key::SecretKey {
		self.0
	}
}

/// Accumulator to compute the sum of blinding factors. Keeps track of each
/// factor as well as the "sign" with which they should be combined.
pub struct BlindSum {
	positive_pubkeys: Vec<Identifier>,
	negative_pubkeys: Vec<Identifier>,
	positive_blinding_factors: Vec<BlindingFactor>,
	negative_blinding_factors: Vec<BlindingFactor>,
}

impl BlindSum {
	/// Creates a new blinding factor sum.
	pub fn new() -> BlindSum {
		BlindSum {
			positive_pubkeys: vec![],
			negative_pubkeys: vec![],
			positive_blinding_factors: vec![],
			negative_blinding_factors: vec![],
		}
	}

	pub fn add_pubkey(mut self, pubkey: Identifier) -> BlindSum {
		self.positive_pubkeys.push(pubkey);
		self
	}

	pub fn sub_pubkey(mut self, pubkey: Identifier) -> BlindSum {
		self.negative_pubkeys.push(pubkey);
		self
	}

	/// Adds the provided key to the sum of blinding factors.
	pub fn add_blinding_factor(mut self, blind: BlindingFactor) -> BlindSum {
		self.positive_blinding_factors.push(blind);
		self
	}

	/// Subtractss the provided key to the sum of blinding factors.
	pub fn sub_blinding_factor(mut self, blind: BlindingFactor) -> BlindSum {
		self.negative_blinding_factors.push(blind);
		self
	}
}

#[derive(Clone, Debug)]
pub struct Keychain {
	secp: Secp256k1,
	extkey: extkey::ExtendedKey,
}

impl Keychain {
	pub fn fingerprint(self) -> Fingerprint {
		self.extkey.fingerprint
	}
}

impl Keychain {
	pub fn from_seed(seed: &[u8]) -> Result<Keychain, Error> {
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		let extkey = extkey::ExtendedKey::from_seed(&secp, seed)?;
		let keychain = Keychain {
			secp: 	secp,
			extkey: extkey,
		};
		Ok(keychain)
	}

	/// For testing - probably not a good idea to use outside of tests.
	pub fn from_random_seed() -> Result<Keychain, Error> {
		let seed: String = thread_rng().gen_ascii_chars().take(16).collect();
		let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());
		Keychain::from_seed(seed.as_bytes())
	}

	pub fn derive_pubkey(&self, derivation: u32) -> Result<Identifier, Error> {
		let extkey = self.extkey.derive(&self.secp, derivation)?;
		Ok(extkey.identifier())
	}

	// TODO - this is a work in progress
	// TODO - smarter lookups - can we cache key_id/fingerprint -> derivation number somehow?
	fn derived_key(&self, pubkey: &Identifier) -> Result<SecretKey, Error> {
		for i in 1..1000 {
			let extkey = self.extkey.derive(&self.secp, i)?;
			if extkey.identifier() == *pubkey {
				return Ok(extkey.key)
			}
		}
		Err(Error::KeyDerivation("cannot find one...".to_string()))
	}

	pub fn commit(&self, amount: u64, pubkey: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(pubkey)?;
		let commit = self.secp.commit(amount, skey)?;
		Ok(commit)
	}

	pub fn switch_commit(&self, pubkey: &Identifier) -> Result<Commitment, Error> {
		let skey = self.derived_key(pubkey)?;
		let commit = self.secp.switch_commit(skey)?;
		Ok(commit)
	}

	pub fn range_proof(
		&self,
		amount: u64,
		pubkey: &Identifier,
		commit: Commitment,
	) -> Result<RangeProof, Error> {
		let skey = self.derived_key(pubkey)?;
		let nonce = self.secp.nonce();
		let range_proof = self.secp.range_proof(0, amount, skey, commit, nonce);
		Ok(range_proof)
	}

	pub fn blind_sum(&self, blind_sum: &BlindSum) -> Result<BlindingFactor, Error> {
		let mut pos_keys: Vec<SecretKey> = blind_sum.positive_pubkeys
			.iter()
			.filter_map(|k| self.derived_key(&k).ok())
			.collect();
		println!("pos_keys - {}", pos_keys.len());

		let mut neg_keys: Vec<SecretKey> = blind_sum.negative_pubkeys
			.iter()
			.filter_map(|k| self.derived_key(&k).ok())
			.collect();
		println!("neg_keys - {}", neg_keys.len());

		pos_keys.extend(&blind_sum.positive_blinding_factors.iter().map(|b| b.0).collect::<Vec<SecretKey>>());
		neg_keys.extend(&blind_sum.negative_blinding_factors.iter().map(|b| b.0).collect::<Vec<SecretKey>>());

		println!("pos_keys all - {}", pos_keys.len());
		println!("neg_keys all - {}", neg_keys.len());

		let blinding = self.secp.blind_sum(pos_keys, neg_keys)?;
		Ok(BlindingFactor(blinding))
		// Err(Error::KeyDerivation("*** not yet implemented ***".to_string()))
	}

	pub fn sign(&self, msg: &Message, pubkey: &Identifier) -> Result<Signature, Error> {
		let skey = self.derived_key(pubkey)?;
		let sig = self.secp.sign(msg, &skey)?;
		Ok(sig)
	}

	pub fn sign_with_blinding(
		&self,
		msg: &Message,
		blinding: &BlindingFactor,
	) -> Result<Signature, Error> {
		let sig = self.secp.sign(msg, &blinding.secret_key())?;
		Ok(sig)
	}

	 pub fn secp(&self) -> &Secp256k1 {
		 &self.secp
	 }
}
