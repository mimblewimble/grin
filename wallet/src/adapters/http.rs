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
use std::collections::HashMap;
use std::sync::Arc;
use util::Mutex;

use api;
use keychain::Keychain;
use libtx::slate::Slate;
use libwallet;
use libwallet::types::*;

#[derive(Clone)]
pub struct HTTPWalletCommAdapter {}

impl HTTPWalletCommAdapter {
	/// Create
	pub fn new() -> HTTPWalletCommAdapter {
		HTTPWalletCommAdapter {}
	}
}

impl WalletCommAdapter for HTTPWalletCommAdapter {
	fn send_tx_sync(&self, dest: &str, slate: &Slate) -> Result<Slate, libwallet::Error> {
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

pub fn start_listener<T: ?Sized, C, K>(
	params: HashMap<String, String>,
	wallet: Arc<Mutex<T>>,
) -> Result<(), libwallet::Error>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	let listen_addr = params.get("api_listen_addr").unwrap();
	let tls_conf = match params.get("certificate") {
		Some(s) => Some(api::TLSConfig::new(
			s.to_owned(),
			params.get("private_key").unwrap().to_owned(),
		)),
		None => None,
	};
	libwallet::controller::foreign_listener(wallet.clone(), &listen_addr, tls_conf)?;
	Ok(())
}
