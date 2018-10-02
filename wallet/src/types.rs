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

use std::cmp::min;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::path::MAIN_SEPARATOR;

use blake2;
use rand::{thread_rng, Rng};

use core::global::ChainTypes;
use error::{Error, ErrorKind};
use failure::ResultExt;
use keychain::Keychain;
use util;
use util::LOGGER;

pub const SEED_FILE: &'static str = "wallet.seed";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalletConfig {
	// Chain parameters (default to Testnet3 if none at the moment)
	pub chain_type: Option<ChainTypes>,
	// The api interface/ip_address that this api server (i.e. this wallet) will run
	// by default this is 127.0.0.1 (and will not accept connections from external clients)
	pub api_listen_interface: String,
	// The port this wallet will run on
	pub api_listen_port: u16,
	/// Location of the secret for basic auth on the Owner API
	pub api_secret_path: Option<String>,
	/// Location of the node api secret for basic auth on the Grin API
	pub node_api_secret_path: Option<String>,
	// The api address of a running server node against which transaction inputs
	// will be checked during send
	pub check_node_api_http_addr: String,
	// The directory in which wallet files are stored
	pub data_file_dir: String,
	/// TLS ceritificate file
	pub tls_certificate_file: Option<String>,
	/// TLS ceritificate password
	pub tls_certificate_pass: Option<String>,
}

impl Default for WalletConfig {
	fn default() -> WalletConfig {
		WalletConfig {
			chain_type: Some(ChainTypes::Testnet3),
			api_listen_interface: "127.0.0.1".to_string(),
			api_listen_port: 13415,
			api_secret_path: Some(".api_secret".to_string()),
			node_api_secret_path: Some(".api_secret".to_string()),
			check_node_api_http_addr: "http://127.0.0.1:13413".to_string(),
			data_file_dir: ".".to_string(),
			tls_certificate_file: None,
			tls_certificate_pass: None,
		}
	}
}

impl WalletConfig {
	pub fn api_listen_addr(&self) -> String {
		format!("{}:{}", self.api_listen_interface, self.api_listen_port)
	}
}

#[derive(Clone, PartialEq)]
pub struct WalletSeed([u8; 32]);

impl WalletSeed {
	pub fn from_bytes(bytes: &[u8]) -> WalletSeed {
		let mut seed = [0; 32];
		for i in 0..min(32, bytes.len()) {
			seed[i] = bytes[i];
		}
		WalletSeed(seed)
	}

	pub fn from_hex(hex: &str) -> Result<WalletSeed, Error> {
		let bytes = util::from_hex(hex.to_string())
			.context(ErrorKind::GenericError("Invalid hex".to_owned()))?;
		Ok(WalletSeed::from_bytes(&bytes))
	}

	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	pub fn derive_keychain<K: Keychain>(&self, password: &str) -> Result<K, Error> {
		let seed = blake2::blake2b::blake2b(64, &password.as_bytes(), &self.0);
		let result = K::from_seed(seed.as_bytes())?;
		Ok(result)
	}

	pub fn init_new() -> WalletSeed {
		let seed: [u8; 32] = thread_rng().gen();
		WalletSeed(seed)
	}

	pub fn init_file(wallet_config: &WalletConfig) -> Result<WalletSeed, Error> {
		// create directory if it doesn't exist
		fs::create_dir_all(&wallet_config.data_file_dir).context(ErrorKind::IO)?;

		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);

		debug!(LOGGER, "Generating wallet seed file at: {}", seed_file_path);

		if Path::new(seed_file_path).exists() {
			Err(ErrorKind::WalletSeedExists)?
		} else {
			let seed = WalletSeed::init_new();
			let mut file = File::create(seed_file_path).context(ErrorKind::IO)?;
			file.write_all(&seed.to_hex().as_bytes())
				.context(ErrorKind::IO)?;
			Ok(seed)
		}
	}

	pub fn from_file(wallet_config: &WalletConfig) -> Result<WalletSeed, Error> {
		// create directory if it doesn't exist
		fs::create_dir_all(&wallet_config.data_file_dir).context(ErrorKind::IO)?;

		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);

		debug!(LOGGER, "Using wallet seed file at: {}", seed_file_path,);

		if Path::new(seed_file_path).exists() {
			let mut file = File::open(seed_file_path).context(ErrorKind::IO)?;
			let mut buffer = String::new();
			file.read_to_string(&mut buffer).context(ErrorKind::IO)?;
			let wallet_seed = WalletSeed::from_hex(&buffer)?;
			Ok(wallet_seed)
		} else {
			error!(
				LOGGER,
				"wallet seed file {} could not be opened (grin wallet init). \
				 Run \"grin wallet init\" to initialize a new wallet.",
				seed_file_path
			);
			Err(ErrorKind::WalletSeedDoesntExist)?
		}
	}
}
