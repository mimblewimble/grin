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

use secp;
use secp::{Message, Secp256k1, Signature};
use secp::key::SecretKey;
use secp::pedersen::{Commitment, RangeProof};
use extkey;

pub use extkey::Identifier;

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

pub struct Keychain {
	secp: Secp256k1,
	extkey: extkey::ExtendedKey,
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

	pub fn derive_pubkey(&self, derivation: u32) -> Result<Identifier, Error> {
		let extkey = self.extkey.derive(&self.secp, derivation)?;
		Ok(extkey.identifier())
	}

	// TODO - this is a work in progress
	// TODO - smarter lookups - can we cache key_id/fingerprint -> derivation number somehow?
	pub fn derived_key(&self, pubkey: &Identifier) -> Result<SecretKey, Error> {
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

	pub fn sign(&self, msg: &Message, pubkey: &Identifier) -> Result<Signature, Error> {
		let skey = self.derived_key(pubkey)?;
		let sig = self.secp.sign(msg, &skey)?;
		Ok(sig)
	}

	 pub fn secp(&self) -> &Secp256k1 {
		 &self.secp
	 }
}
