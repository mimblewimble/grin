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
use libwallet::api::APIOwner;

/// Wallet controller
pub struct Context {}

impl Context {
	/// Instantiate wallet and API for a single-use (command line) call
	/// Return a function containing a loaded API context to call
	pub fn owner_single_use<F>(config: WalletConfig, passphrase: &str, f: F) -> Result<(), Error>
	where
		F: FnOnce(&mut APIOwner<FileWallet>) -> Result<(), Error>,
	{
		let mut wallet = Context::load_wallet(config, passphrase)?;
		let mut api = APIOwner::new(&mut wallet);
		f(&mut api)?;
		Ok(())
	}

	/// Listener version, providing same API but listening for requests on a
	/// port and wrapping the calls
	pub fn owner_listener<F>(config: WalletConfig, passphrase: &str, f: &mut F) -> Result<(), Error>
	where
		F: FnMut(&mut APIOwner<FileWallet>) -> Result<(), Error>,
	{
		let mut wallet = Context::load_wallet(config, passphrase)?;
		f(&mut APIOwner::new(&mut wallet))?;
		Ok(())
	}

	// load up wallet
	fn load_wallet(config: WalletConfig, passphrase: &str) -> Result<FileWallet, Error> {
		let wallet_seed = WalletSeed::from_file(&config)?;
		let keychain = wallet_seed.derive_keychain(passphrase)?;
		Ok(FileWallet::new(config, keychain)?)
	}
}
