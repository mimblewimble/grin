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

/// Implementation of the Keychain trait based on an extended key derivation
/// scheme.
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use blake2;

use extkey_bip32::{BIP32GrinHasher, ExtendedPrivKey};
use types::{BlindSum, BlindingFactor, Error, ExtKeychainPath, Identifier, Keychain};
use util::secp::key::SecretKey;
use util::secp::pedersen::Commitment;
use util::secp::{self, Message, Secp256k1, Signature};

#[derive(Clone, Debug)]
pub struct ExtKeychain {
	secp: Secp256k1,
	master: ExtendedPrivKey,
}

impl Keychain for ExtKeychain {
	fn from_seed(seed: &[u8]) -> Result<ExtKeychain, Error> {
		let mut h = BIP32GrinHasher::new();
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		let master = ExtendedPrivKey::new_master(&secp, &mut h, seed)?;
		let keychain = ExtKeychain {
			secp: secp,
			master: master,
		};
		Ok(keychain)
	}

	/// For testing - probably not a good idea to use outside of tests.
	fn from_random_seed() -> Result<ExtKeychain, Error> {
		let seed: String = thread_rng().sample_iter(&Alphanumeric).take(16).collect();
		let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());
		ExtKeychain::from_seed(seed.as_bytes())
	}

	fn root_key_id() -> Identifier {
		ExtKeychainPath::new(0, 0, 0, 0, 0).to_identifier()
	}

	fn derive_key_id(depth: u8, d1: u32, d2: u32, d3: u32, d4: u32) -> Identifier {
		ExtKeychainPath::new(depth, d1, d2, d3, d4).to_identifier()
	}

	fn derive_key(&self, id: &Identifier) -> Result<ExtendedPrivKey, Error> {
		let mut h = BIP32GrinHasher::new();
		let p = id.to_path();
		let mut sk = self.master;
		for i in 0..p.depth {
			sk = sk.ckd_priv(&self.secp, &mut h, p.path[i as usize])?;
		}
		Ok(sk)
	}

	fn commit(&self, amount: u64, id: &Identifier) -> Result<Commitment, Error> {
		let key = self.derive_key(id)?;
		let commit = self.secp.commit(amount, key.secret_key)?;
		Ok(commit)
	}

	fn blind_sum(&self, blind_sum: &BlindSum) -> Result<BlindingFactor, Error> {
		let mut pos_keys: Vec<SecretKey> = blind_sum
			.positive_key_ids
			.iter()
			.filter_map(|k| {
				let res = self.derive_key(&Identifier::from_path(&k));
				if let Ok(s) = res {
					Some(s.secret_key)
				} else {
					None
				}
			})
			.collect();

		let mut neg_keys: Vec<SecretKey> = blind_sum
			.negative_key_ids
			.iter()
			.filter_map(|k| {
				let res = self.derive_key(&Identifier::from_path(&k));
				if let Ok(s) = res {
					Some(s.secret_key)
				} else {
					None
				}
			})
			.collect();

		pos_keys.extend(
			&blind_sum
				.positive_blinding_factors
				.iter()
				.filter_map(|b| b.secret_key(&self.secp).ok())
				.collect::<Vec<SecretKey>>(),
		);

		neg_keys.extend(
			&blind_sum
				.negative_blinding_factors
				.iter()
				.filter_map(|b| b.secret_key(&self.secp).ok())
				.collect::<Vec<SecretKey>>(),
		);

		let sum = self.secp.blind_sum(pos_keys, neg_keys)?;
		Ok(BlindingFactor::from_secret_key(sum))
	}

	fn sign(&self, msg: &Message, id: &Identifier) -> Result<Signature, Error> {
		let skey = self.derive_key(id)?;
		let sig = self.secp.sign(msg, &skey.secret_key)?;
		Ok(sig)
	}

	fn sign_with_blinding(
		&self,
		msg: &Message,
		blinding: &BlindingFactor,
	) -> Result<Signature, Error> {
		let skey = &blinding.secret_key(&self.secp)?;
		let sig = self.secp.sign(msg, &skey)?;
		Ok(sig)
	}

	fn secp(&self) -> &Secp256k1 {
		&self.secp
	}
}

#[cfg(test)]
mod test {
	use keychain::ExtKeychain;
	use types::{BlindSum, BlindingFactor, ExtKeychainPath, Keychain};
	use util::secp;
	use util::secp::key::SecretKey;

	#[test]
	fn test_key_derivation() {
		let keychain = ExtKeychain::from_random_seed().unwrap();
		let secp = keychain.secp();

		let path = ExtKeychainPath::new(1, 1, 0, 0, 0);
		let key_id = path.to_identifier();

		let msg_bytes = [0; 32];
		let msg = secp::Message::from_slice(&msg_bytes[..]).unwrap();

		// now create a zero commitment using the key on the keychain associated with
		// the key_id
		let commit = keychain.commit(0, &key_id).unwrap();

		// now check we can use our key to verify a signature from this zero commitment
		let sig = keychain.sign(&msg, &key_id).unwrap();
		secp.verify_from_commit(&msg, &sig, &commit).unwrap();
	}

	// We plan to "offset" the key used in the kernel commitment
	// so we are going to be doing some key addition/subtraction.
	// This test is mainly to demonstrate that idea that summing commitments
	// and summing the keys used to commit to 0 have the same result.
	#[test]
	fn secret_key_addition() {
		let keychain = ExtKeychain::from_random_seed().unwrap();

		let skey1 = SecretKey::from_slice(
			&keychain.secp,
			&[
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 1,
			],
		).unwrap();

		let skey2 = SecretKey::from_slice(
			&keychain.secp,
			&[
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 2,
			],
		).unwrap();

		// adding secret keys 1 and 2 to give secret key 3
		let mut skey3 = skey1.clone();
		let _ = skey3.add_assign(&keychain.secp, &skey2).unwrap();

		// create commitments for secret keys 1, 2 and 3
		// all committing to the value 0 (which is what we do for tx_kernels)
		let commit_1 = keychain.secp.commit(0, skey1).unwrap();
		let commit_2 = keychain.secp.commit(0, skey2).unwrap();
		let commit_3 = keychain.secp.commit(0, skey3).unwrap();

		// now sum commitments for keys 1 and 2
		let sum = keychain
			.secp
			.commit_sum(vec![commit_1.clone(), commit_2.clone()], vec![])
			.unwrap();

		// confirm the commitment for key 3 matches the sum of the commitments 1 and 2
		assert_eq!(sum, commit_3);

		// now check we can sum keys up using keychain.blind_sum()
		// in the same way (convenience function)
		assert_eq!(
			keychain
				.blind_sum(
					&BlindSum::new()
						.add_blinding_factor(BlindingFactor::from_secret_key(skey1))
						.add_blinding_factor(BlindingFactor::from_secret_key(skey2))
				)
				.unwrap(),
			BlindingFactor::from_secret_key(skey3),
		);
	}
}
