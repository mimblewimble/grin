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

/// File Output 'plugin' implementation
use std::fs::File;
use std::io::{Read, Write};

use serde_json as json;
use std::collections::HashMap;

use core::libtx::slate::Slate;
use libwallet::{Error, ErrorKind};
use {WalletCommAdapter, WalletConfig};

#[derive(Clone)]
pub struct FileWalletCommAdapter {}

impl FileWalletCommAdapter {
	/// Create
	pub fn new() -> Box<WalletCommAdapter> {
		Box::new(FileWalletCommAdapter {})
	}
}

impl WalletCommAdapter for FileWalletCommAdapter {
	fn supports_sync(&self) -> bool {
		false
	}

	fn send_tx_sync(&self, _dest: &str, _slate: &Slate) -> Result<Slate, Error> {
		unimplemented!();
	}

	fn send_tx_async(&self, dest: &str, slate: &Slate) -> Result<(), Error> {
		let mut pub_tx = File::create(dest)?;
		pub_tx.write_all(json::to_string(&slate).unwrap().as_bytes())?;
		pub_tx.sync_all()?;
		Ok(())
	}

	fn receive_tx_async(&self, params: &str) -> Result<Slate, Error> {
		let mut pub_tx_f = File::open(params)?;
		let mut content = String::new();
		pub_tx_f.read_to_string(&mut content)?;
		Ok(json::from_str(&content).map_err(|_| ErrorKind::Format)?)
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
