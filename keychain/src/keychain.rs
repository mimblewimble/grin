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
use secp::pedersen::{Commitment, ProofMessage, ProofInfo, RangeProof};
use blake2;

use blind::{BlindingFactor, BlindSum};
use extkey::{self, Fingerprint, Identifier};


#[derive(PartialEq, Eq, Clone, Debug)]
pub enum Error {
	ExtendedKey(extkey::Error),
	Secp(secp::Error),
	KeyDerivation(String),
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<extkey::Error> for Error {
	fn from(e: extkey::Error) -> Error {
		Error::ExtendedKey(e)
	}
}


#[derive(Clone, Debug)]
pub struct Keychain {
	secp: Secp256k1,
	extkey: extkey::ExtendedKey,

	/// for tests and burn only, associate the zero fingerprint to a known
	/// dummy private key
	pub enable_burn_key: bool,
}

impl Keychain {
	pub fn fingerprint(&self) -> Fingerprint {
		self.extkey.fingerprint.clone()
	}

	pub fn from_seed(seed: &[u8]) -> Result<Keychain, Error> {
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		let extkey = extkey::ExtendedKey::from_seed(&secp, seed)?;
		let keychain = Keychain {
			secp: secp,
			extkey: extkey,
			enable_burn_key: false,
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
		let pubkey = extkey.identifier(&self.secp)?;
		Ok(pubkey)
	}

	// TODO - this is a work in progress
	// TODO - smarter lookups - can we cache key_id/fingerprint -> derivation
	// number somehow?
	fn derived_key(&self, pubkey: &Identifier) -> Result<SecretKey, Error> {
		if self.enable_burn_key {
			// for tests and burn only, associate the zero fingerprint to a known
			// dummy private key
			if pubkey.fingerprint().to_string() == "00000000" {
				return Ok(SecretKey::from_slice(&self.secp, &[1; 32])?);
			}
		}
		for i in 1..10000 {
			let extkey = self.extkey.derive(&self.secp, i)?;
			if extkey.identifier(&self.secp)? == *pubkey {
				return Ok(extkey.key);
			}
		}
		Err(Error::KeyDerivation(format!("cannot find extkey for {}", pubkey.fingerprint())))
	}

	// TODO - clean this and derived_key up, rename them?
	// TODO - maybe wallet deals exclusively with pubkeys and not derivations - this leaks?
	pub fn derivation_from_pubkey(&self, pubkey: &Identifier) -> Result<u32, Error> {
		for i in 1..10000 {
			let extkey = self.extkey.derive(&self.secp, i)?;
			if extkey.identifier(&self.secp)? == *pubkey {
				return Ok(extkey.n_child);
			}
		}
		Err(Error::KeyDerivation(format!("cannot find extkey for {}", pubkey.fingerprint())))
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
		msg: ProofMessage,
	) -> Result<RangeProof, Error> {
		let skey = self.derived_key(pubkey)?;
		let range_proof = self.secp.range_proof(0, amount, skey, commit, msg);
		Ok(range_proof)
	}

	pub fn rewind_range_proof(
		&self,
		pubkey: &Identifier,
		commit: Commitment,
		proof: RangeProof,
	) -> Result<ProofInfo, Error> {
		let nonce = self.derived_key(pubkey)?;
		Ok(self.secp.rewind_range_proof(commit, proof, nonce))
	}

	pub fn blind_sum(&self, blind_sum: &BlindSum) -> Result<BlindingFactor, Error> {
		let mut pos_keys: Vec<SecretKey> = blind_sum
			.positive_pubkeys
			.iter()
			.filter_map(|k| self.derived_key(&k).ok())
			.collect();

		let mut neg_keys: Vec<SecretKey> = blind_sum
			.negative_pubkeys
			.iter()
			.filter_map(|k| self.derived_key(&k).ok())
			.collect();

		pos_keys.extend(&blind_sum
			.positive_blinding_factors
			.iter()
			.map(|b| b.secret_key())
			.collect::<Vec<SecretKey>>());

		neg_keys.extend(&blind_sum
			.negative_blinding_factors
			.iter()
			.map(|b| b.secret_key())
			.collect::<Vec<SecretKey>>());

		let blinding = self.secp.blind_sum(pos_keys, neg_keys)?;
		Ok(BlindingFactor::new(blinding))
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

#[cfg(test)]
mod test {
	use keychain::Keychain;
	use secp;
	use secp::pedersen::ProofMessage;

	#[test]
	fn test_key_derivation() {
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		let keychain = Keychain::from_random_seed().unwrap();

		// use the keychain to derive a "pubkey" based on the underlying seed
		let pubkey = keychain.derive_pubkey(1).unwrap();

		let msg_bytes = [0; 32];
		let msg = secp::Message::from_slice(&msg_bytes[..]).unwrap();

		// now create a zero commitment using the key on the keychain associated with
		// the pubkey
		let commit = keychain.commit(0, &pubkey).unwrap();

		// now check we can use our key to verify a signature from this zero commitment
		let sig = keychain.sign(&msg, &pubkey).unwrap();
		secp.verify_from_commit(&msg, &sig, &commit).unwrap();
	}

	#[test]
	fn test_rewind_range_proof() {
		let keychain = Keychain::from_random_seed().unwrap();
		let pubkey = keychain.derive_pubkey(1).unwrap();
		let commit = keychain.commit(5, &pubkey).unwrap();
		let msg = ProofMessage::empty();

		let proof = keychain.range_proof(5, &pubkey, commit, msg).unwrap();
		let proof_info = keychain.rewind_range_proof(&pubkey, commit, proof).unwrap();

		assert_eq!(proof_info.success, true);
		assert_eq!(proof_info.value, 5);

		// now check the recovered message is "empty" (but not truncated) i.e. all zeroes
		assert_eq!(
			proof_info.message,
			secp::pedersen::ProofMessage::from_bytes(&[0; secp::constants::PROOF_MSG_SIZE])
		);

		let pubkey2 = keychain.derive_pubkey(2).unwrap();

		// cannot rewind with a different nonce
		let proof_info = keychain.rewind_range_proof(&pubkey2, commit, proof).unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with a commitment to the same value using a different key
		let commit2 = keychain.commit(5, &pubkey2).unwrap();
		let proof_info = keychain.rewind_range_proof(&pubkey, commit2, proof).unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);

		// cannot rewind with a commitment to a different value
		let commit3 = keychain.commit(4, &pubkey).unwrap();
		let proof_info = keychain.rewind_range_proof(&pubkey, commit3, proof).unwrap();
		assert_eq!(proof_info.success, false);
		assert_eq!(proof_info.value, 0);
	}
}
