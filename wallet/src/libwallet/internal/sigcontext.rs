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
//! Signature context holder helper (may be removed or replaced eventually)
use keychain::Identifier;
use libtx::aggsig;
use util::secp::key::{PublicKey, SecretKey};
use util::secp::{self, Secp256k1};

#[derive(Clone, Debug)]
/// Holds the context for a single aggsig transaction
pub struct Context {
	/// Secret key (of which public is shared)
	pub sec_key: SecretKey,
	/// Secret nonce (of which public is shared)
	/// (basically a SecretKey)
	pub sec_nonce: SecretKey,
	/// store my outputs between invocations
	pub output_ids: Vec<Identifier>,
	/// store my inputs
	pub input_ids: Vec<Identifier>,
	/// store the calculated fee
	pub fee: u64,
}

impl Context {
	/// Create a new context with defaults
	pub fn new(secp: &secp::Secp256k1, sec_key: SecretKey) -> Context {
		Context {
			sec_key: sec_key,
			sec_nonce: aggsig::create_secnonce(secp).unwrap(),
			input_ids: vec![],
			output_ids: vec![],
			fee: 0,
		}
	}
}

impl Context {
	/// Tracks an output contributing to my excess value (if it needs to
	/// be kept between invocations
	pub fn add_output(&mut self, output_id: &Identifier) {
		self.output_ids.push(output_id.clone());
	}

	/// Returns all stored outputs
	pub fn get_outputs(&self) -> Vec<Identifier> {
		self.output_ids.clone()
	}

	/// Tracks IDs of my inputs into the transaction
	/// be kept between invocations
	pub fn add_input(&mut self, input_id: &Identifier) {
		self.input_ids.push(input_id.clone());
	}

	/// Returns all stored input identifiers
	pub fn get_inputs(&self) -> Vec<Identifier> {
		self.input_ids.clone()
	}

	/// Returns private key, private nonce
	pub fn get_private_keys(&self) -> (SecretKey, SecretKey) {
		(self.sec_key.clone(), self.sec_nonce.clone())
	}

	/// Returns public key, public nonce
	pub fn get_public_keys(&self, secp: &Secp256k1) -> (PublicKey, PublicKey) {
		(
			PublicKey::from_secret_key(secp, &self.sec_key).unwrap(),
			PublicKey::from_secret_key(secp, &self.sec_nonce).unwrap(),
		)
	}
}
