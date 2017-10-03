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


use secp::{self, Secp256k1};
use extkey::Identifier;
use keychain::Error;

#[derive(Clone, Debug)]
pub struct BlindingFactor(secp::key::SecretKey);

impl BlindingFactor {
	pub fn new(secret_key: secp::key::SecretKey) -> BlindingFactor {
		BlindingFactor(secret_key)
	}

	pub fn secret_key(&self) -> secp::key::SecretKey {
		self.0
	}

	pub fn from_slice(secp: &Secp256k1, data: &[u8]) -> Result<BlindingFactor, Error> {
		Ok(BlindingFactor(
			secp::key::SecretKey::from_slice(&secp, data)?,
		))
	}
}

/// Accumulator to compute the sum of blinding factors. Keeps track of each
/// factor as well as the "sign" with which they should be combined.
pub struct BlindSum {
	pub positive_pubkeys: Vec<Identifier>,
	pub negative_pubkeys: Vec<Identifier>,
	pub positive_blinding_factors: Vec<BlindingFactor>,
	pub negative_blinding_factors: Vec<BlindingFactor>,
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
