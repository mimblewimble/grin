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

//! Controller for wallet.. instantiates and handles listeners (or single-run
//! invocations) as needed.
//! Still experimental

use error::Error;
use file_wallet::{FileWallet, WalletConfig, WalletSeed};
use libwallet::api::APIInternal;

/// Wallet controller
pub struct Context {}

impl Context {
	/// Instantiate wallet and API for a single-use (command line) call
	/// Return a function containing a loaded API context to call
	pub fn internal_single_use<F>(config: WalletConfig, passphrase: &str, f: F) -> Result<(), Error>
	where
		F: FnOnce(&mut APIInternal<FileWallet>) -> Result<(), Error>,
	{
		// Load up wallet and keychain
		let wallet_seed = WalletSeed::from_file(&config)?;

		let keychain = wallet_seed.derive_keychain(passphrase)?;

		let mut wallet = FileWallet::new(config.clone(), keychain)?;

		// Instantiate API
		let mut api = APIInternal::new(&mut wallet);
		f(&mut api)?;
		Ok(())
	}

	/// Listener version, providing same API but listening for requests on a
	/// port and wrapping the calls
	pub fn tbd() {}
}
