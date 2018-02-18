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
/// Encapsulate a secret key for the blind_sum operation

use std::cmp::min;
use rand::thread_rng;

use extkey::Identifier;
use keychain::Error;
use util;
use util::secp::{self, Secp256k1};
use util::secp::constants::SECRET_KEY_SIZE;


#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlindingFactor([u8; SECRET_KEY_SIZE]);

impl AsRef<[u8]> for BlindingFactor {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl BlindingFactor {
	pub fn from_secret_key(skey: secp::key::SecretKey) -> BlindingFactor {
		BlindingFactor::from_slice(&skey.as_ref())
	}

	pub fn from_slice(data: &[u8]) -> BlindingFactor {
		let mut blind = [0; SECRET_KEY_SIZE];
		for i in 0..min(SECRET_KEY_SIZE, data.len()) {
			blind[i] = data[i];
		}
		BlindingFactor(blind)
	}

	pub fn zero() -> BlindingFactor {
		BlindingFactor::from_secret_key(secp::key::ZERO_KEY)
	}

	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	pub fn from_hex(hex: &str) -> Result<BlindingFactor, Error> {
		let bytes = util::from_hex(hex.to_string()).unwrap();
		Ok(BlindingFactor::from_slice(&bytes))
	}

	pub fn secret_key(&self, secp: &Secp256k1) -> Result<secp::key::SecretKey, Error> {
		if *self == BlindingFactor::zero() {
			// TODO - need this currently for tx tests
			// the "zero" secret key is not actually a valid secret_key
			// and secp lib checks this
			Ok(secp::key::ZERO_KEY)
		} else {
			secp::key::SecretKey::from_slice(secp, &self.0)
				.map_err(|e| Error::Secp(e))
		}
	}

	/// Split a blinding_factor (aka secret_key) into a pair of blinding_factors.
	/// We use one of these (k1) to sign the tx_kernel (k1G)
	/// and the other gets aggregated in the block_header as the "offset".
	/// This prevents an actor from being able to sum a set of inputs, outputs and kernels
	/// from a block to identify and reconstruct a particular tx from a block.
	/// You would need both k1, k2 to do this.
	pub fn split(&self, secp: &Secp256k1) -> Result<SplitBlindingFactor, Error> {
		let skey_1 = secp::key::SecretKey::new(secp, &mut thread_rng());

		// use blind_sum to subtract skey_1 from our key (to give k = k1 + k2)
		let skey = self.secret_key(secp)?;
		let skey_2 = secp.blind_sum(vec![skey], vec![skey_1])?;

		let blind_1 = BlindingFactor::from_secret_key(skey_1);
		let blind_2 = BlindingFactor::from_secret_key(skey_2);

		Ok(SplitBlindingFactor {
			blind_1,
			blind_2,
		})
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SplitBlindingFactor {
	pub blind_1: BlindingFactor,
	pub blind_2: BlindingFactor,
}

/// Accumulator to compute the sum of blinding factors. Keeps track of each
/// factor as well as the "sign" with which they should be combined.
#[derive(Clone, Debug, PartialEq)]
pub struct BlindSum {
	pub positive_key_ids: Vec<Identifier>,
	pub negative_key_ids: Vec<Identifier>,
	pub positive_blinding_factors: Vec<BlindingFactor>,
	pub negative_blinding_factors: Vec<BlindingFactor>,
}

impl BlindSum {
	/// Creates a new blinding factor sum.
	pub fn new() -> BlindSum {
		BlindSum {
			positive_key_ids: vec![],
			negative_key_ids: vec![],
			positive_blinding_factors: vec![],
			negative_blinding_factors: vec![],
		}
	}

	pub fn add_key_id(mut self, key_id: Identifier) -> BlindSum {
		self.positive_key_ids.push(key_id);
		self
	}

	pub fn sub_key_id(mut self, key_id: Identifier) -> BlindSum {
		self.negative_key_ids.push(key_id);
		self
	}

	/// Adds the provided key to the sum of blinding factors.
	pub fn add_blinding_factor(mut self, blind: BlindingFactor) -> BlindSum {
		self.positive_blinding_factors.push(blind);
		self
	}

	/// Subtracts the provided key to the sum of blinding factors.
	pub fn sub_blinding_factor(mut self, blind: BlindingFactor) -> BlindSum {
		self.negative_blinding_factors.push(blind);
		self
	}
}

#[cfg(test)]
mod test {
	use rand::thread_rng;

	use blind::BlindingFactor;
	use util::secp::Secp256k1;
	use util::secp::key::{SecretKey, ZERO_KEY};

	#[test]
	fn split_blinding_factor() {
		let secp = Secp256k1::new();
		let skey_in = SecretKey::new(&secp, &mut thread_rng());
		let blind = BlindingFactor::from_secret_key(skey_in);
		let split = blind.split(&secp).unwrap();

		// split a key, sum the split keys and confirm the sum matches the original key
		let mut skey_sum = split.blind_1.secret_key(&secp).unwrap();
		let skey_2 = split.blind_2.secret_key(&secp).unwrap();
		let _ = skey_sum.add_assign(&secp, &skey_2).unwrap();
		assert_eq!(skey_in, skey_sum);
	}

	// Sanity check that we can add the zero key to a secret key and it is still
	// the same key that we started with (k + 0 = k)
	#[test]
	fn zero_key_addition() {
		let secp = Secp256k1::new();
		let skey_in = SecretKey::new(&secp, &mut thread_rng());
		let skey_zero = ZERO_KEY;

		let mut skey_out = skey_in.clone();
		let _ = skey_out.add_assign(&secp, &skey_zero).unwrap();

		assert_eq!(skey_in, skey_out);
	}
}
