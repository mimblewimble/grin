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

/// HTTP Wallet 'plugin' implementation
use failure::ResultExt;

use api;
use error::{Error, ErrorKind};
use libtx::slate::Slate;
use libwallet;
use libwallet::types::*;

#[derive(Clone)]
pub struct HTTPWalletToWalletClient {}

impl HTTPWalletToWalletClient {
	/// Create a new client that will communicate other wallets
	pub fn new() -> HTTPWalletToWalletClient {
		HTTPWalletToWalletClient {}
	}
}

impl WalletToWalletClient for HTTPWalletToWalletClient {
	/// Send the slate to a listening wallet instance
	fn send_tx_slate(&self, dest: &str, slate: &Slate) -> Result<Slate, libwallet::Error> {
		if &dest[..4] != "http" {
			let err_str = format!(
				"dest formatted as {} but send -d expected stdout or http://IP:port",
				dest
			);
			error!("{}", err_str,);
			Err(libwallet::ErrorKind::Uri)?
		}
		let url = format!("{}/v1/wallet/foreign/receive_tx", dest);
		debug!("Posting transaction slate to {}", url);

		let res = api::client::post(url.as_str(), None, slate).context(
			libwallet::ErrorKind::ClientCallback("Posting transaction slate"),
		)?;
		Ok(res)
	}
}
