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

use api;
use controller;
use core::libtx::slate::Slate;
use libwallet::{Error, ErrorKind};
use {instantiate_wallet, HTTPNodeClient, WalletCommAdapter, WalletConfig};

#[derive(Clone)]
pub struct HTTPWalletCommAdapter {}

impl HTTPWalletCommAdapter {
	/// Create
	pub fn new() -> Box<WalletCommAdapter> {
		Box::new(HTTPWalletCommAdapter {})
	}
}

impl WalletCommAdapter for HTTPWalletCommAdapter {
	fn supports_sync(&self) -> bool {
		true
	}

	fn send_tx_sync(&self, dest: &str, slate: &Slate) -> Result<Slate, Error> {
		if &dest[..4] != "http" {
			let err_str = format!(
				"dest formatted as {} but send -d expected stdout or http://IP:port",
				dest
			);
			error!("{}", err_str,);
			Err(ErrorKind::Uri)?
		}
		let url = format!("{}/v1/wallet/foreign/receive_tx", dest);
		debug!("Posting transaction slate to {}", url);

		let res = api::client::post(url.as_str(), None, slate)
			.context(ErrorKind::ClientCallback("Posting transaction slate"))?;
		Ok(res)
	}

	fn send_tx_async(&self, _dest: &str, _slate: &Slate) -> Result<(), Error> {
		unimplemented!();
	}

	fn receive_tx_async(&self, _params: &str) -> Result<Slate, Error> {
		unimplemented!();
	}

	fn listen(
		&self,
		params: HashMap<String, String>,
		config: WalletConfig,
		passphrase: &str,
		account: &str,
		node_api_secret: Option<String>,
	) -> Result<(), Error> {
		let node_client = HTTPNodeClient::new(&config.check_node_api_http_addr, node_api_secret);
		let wallet = instantiate_wallet(config.clone(), node_client, passphrase, account)
			.context(ErrorKind::WalletSeedDecryption)?;
		let listen_addr = params.get("api_listen_addr").unwrap();
		let tls_conf = match params.get("certificate") {
			Some(s) => Some(api::TLSConfig::new(
				s.to_owned(),
				params.get("private_key").unwrap().to_owned(),
			)),
			None => None,
		};
		controller::foreign_listener(wallet.clone(), &listen_addr, tls_conf)?;
		Ok(())
	}
}
