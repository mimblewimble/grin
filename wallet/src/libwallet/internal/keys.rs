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

//! Wallet key management functions
use keychain::{Identifier, Keychain};
use libwallet::error::Error;
use libwallet::types::{WalletBackend, WalletClient};

/// Get next available key in the wallet for a given parent
pub fn next_available_key<T: ?Sized, C, K>(wallet: &mut T) -> Result<Identifier, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let child = wallet.next_child()?;
	Ok(child)
}

/// Retrieve an existing key from a wallet
pub fn retrieve_existing_key<T: ?Sized, C, K>(
	wallet: &T,
	key_id: Identifier,
) -> Result<(Identifier, u32), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let existing = wallet.get(&key_id)?;
	let key_id = existing.key_id.clone();
	let derivation = existing.n_child;
	Ok((key_id, derivation))
}
