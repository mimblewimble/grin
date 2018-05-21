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

//! Grin Wallet specific key management functions
use rand::thread_rng;
use uuid::Uuid;

use core::core::{amount_to_hr_string, Committed, Transaction};
use libwallet::{aggsig, build};
use keychain::{BlindSum, BlindingFactor, Identifier, Keychain};
use types::*;
use util::{secp, LOGGER};
use util::secp::key::{PublicKey, SecretKey};
use util::secp::Signature;
use failure::ResultExt;

/// Get next available key in the wallet
pub fn next_available_key(wallet_data: &WalletData, keychain: &Keychain) -> (Identifier, u32) {
	let root_key_id = keychain.root_key_id();
	let derivation = wallet_data.next_child(root_key_id.clone());
	let key_id = keychain.derive_key_id(derivation).unwrap();
	(key_id, derivation)
}

/// Retrieve an existing key from a wallet
pub fn retrieve_existing_key(wallet_data: &WalletData, key_id: Identifier) -> (Identifier, u32) {
	if let Some(existing) = wallet_data.get_output(&key_id) {
		let key_id = existing.key_id.clone();
		let derivation = existing.n_child;
		(key_id, derivation)
	} else {
		panic!("should never happen");
	}
}
