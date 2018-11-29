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

mod file;
mod http;
mod null;

pub use self::file::FileWalletCommAdapter;
pub use self::http::HTTPWalletCommAdapter;
pub use self::null::NullWalletCommAdapter;

use std::collections::HashMap;

use libtx::slate::Slate;
use libwallet::Error;
use WalletConfig;

/// Encapsulate wallet to wallet communication functions
pub trait WalletCommAdapter {
	/// Whether this adapter supports sync mode
	fn supports_sync(&self) -> bool;

	/// Send a transaction slate to another listening wallet and return result
	/// TODO: Probably need a slate wrapper type
	fn send_tx_sync(&self, addr: &str, slate: &Slate) -> Result<Slate, Error>;

	/// Send a transaction asynchronously (result will be returned via the listener)
	fn send_tx_async(&self, addr: &str, slate: &Slate) -> Result<(), Error>;

	/// Receive a transaction async. (Actually just read it from wherever and return the slate)
	fn receive_tx_async(&self, params: &str) -> Result<Slate, Error>;

	/// Start a listener, passing received messages to the wallet api directly
	/// Takes a wallet config for now to avoid needing all sorts of awkward
	/// type parameters on this trait
	fn listen(
		&self,
		params: HashMap<String, String>,
		config: WalletConfig,
		passphrase: &str,
		account: &str,
		node_api_secret: Option<String>,
	) -> Result<(), Error>;
}
