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

/// Null Output 'plugin' implementation
use core::libtx::slate::Slate;
use libwallet::Error;
use {WalletCommAdapter, WalletConfig};

use std::collections::HashMap;

#[derive(Clone)]
pub struct NullWalletCommAdapter {}

impl NullWalletCommAdapter {
	/// Create
	pub fn new() -> Box<NullWalletCommAdapter> {
		Box::new(NullWalletCommAdapter {})
	}
}

impl WalletCommAdapter for NullWalletCommAdapter {
	fn supports_sync(&self) -> bool {
		true
	}

	fn send_tx_sync(&self, _dest: &str, slate: &Slate) -> Result<Slate, Error> {
		Ok(slate.clone())
	}

	fn send_tx_async(&self, _dest: &str, _slate: &Slate) -> Result<(), Error> {
		Ok(())
	}

	fn receive_tx_async(&self, _params: &str) -> Result<Slate, Error> {
		unimplemented!();
	}

	fn listen(
		&self,
		_params: HashMap<String, String>,
		_config: WalletConfig,
		_passphrase: &str,
		_account: &str,
		_node_api_secret: Option<String>,
	) -> Result<(), Error> {
		unimplemented!();
	}
}
