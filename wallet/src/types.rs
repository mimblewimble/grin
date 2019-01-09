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

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::path::MAIN_SEPARATOR;

use crate::blake2;
use rand::{thread_rng, Rng};
use serde_json;

use ring::aead;
use ring::{digest, pbkdf2};

use crate::core::global::ChainTypes;
use crate::error::{Error, ErrorKind};
use crate::keychain::{mnemonic, Keychain};
use crate::util;
use failure::ResultExt;

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
	// Whether to include foreign API endpoints on the Owner API
	pub owner_api_include_foreign: Option<bool>,
	// The directory in which wallet files are stored
	pub data_file_dir: String,
	/// TLS certificate file
	pub tls_certificate_file: Option<String>,
	/// TLS certificate private key file
	pub tls_certificate_key: Option<String>,
	/// Whether to use the black background color scheme for command line
	/// if enabled, wallet command output color will be suitable for black background terminal
	pub dark_background_color_scheme: Option<bool>,
	// The exploding lifetime (minutes) for keybase notification on coins received
	pub keybase_notify_ttl: Option<u16>,
}

impl Default for WalletConfig {
	fn default() -> WalletConfig {
		WalletConfig {
			chain_type: Some(ChainTypes::Floonet),
			api_listen_interface: "127.0.0.1".to_string(),
			api_listen_port: 3415,
			api_secret_path: Some(".api_secret".to_string()),
			node_api_secret_path: Some(".api_secret".to_string()),
			check_node_api_http_addr: "http://127.0.0.1:3413".to_string(),
			owner_api_include_foreign: Some(false),
			data_file_dir: ".".to_string(),
			tls_certificate_file: None,
			tls_certificate_key: None,
			dark_background_color_scheme: Some(true),
			keybase_notify_ttl: Some(1440),
		}
	}
}

impl WalletConfig {
	pub fn api_listen_addr(&self) -> String {
		format!("{}:{}", self.api_listen_interface, self.api_listen_port)
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct WalletSeed(Vec<u8>);

impl WalletSeed {
	pub fn from_bytes(bytes: &[u8]) -> WalletSeed {
		WalletSeed(bytes.to_vec())
	}

	pub fn from_mnemonic(word_list: &str) -> Result<WalletSeed, Error> {
		let res = mnemonic::to_entropy(word_list);
		match res {
			Ok(s) => Ok(WalletSeed::from_bytes(&s)),
			Err(_) => Err(ErrorKind::Mnemonic.into()),
		}
	}

	pub fn from_hex(hex: &str) -> Result<WalletSeed, Error> {
		let bytes = util::from_hex(hex.to_string())
			.context(ErrorKind::GenericError("Invalid hex".to_owned()))?;
		Ok(WalletSeed::from_bytes(&bytes))
	}

	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	pub fn to_mnemonic(&self) -> Result<String, Error> {
		let result = mnemonic::from_entropy(&self.0);
		match result {
			Ok(r) => Ok(r),
			Err(_) => Err(ErrorKind::Mnemonic.into()),
		}
	}

	pub fn derive_keychain_old(old_wallet_seed: [u8; 32], password: &str) -> Vec<u8> {
		let seed = blake2::blake2b::blake2b(64, password.as_bytes(), &old_wallet_seed);
		seed.as_bytes().to_vec()
	}

	pub fn derive_keychain<K: Keychain>(&self, is_floonet: bool) -> Result<K, Error> {
		let result = K::from_seed(&self.0, is_floonet)?;
		Ok(result)
	}

	pub fn init_new(seed_length: usize) -> WalletSeed {
		let mut seed: Vec<u8> = vec![];
		let mut rng = thread_rng();
		for _ in 0..seed_length {
			seed.push(rng.gen());
		}
		WalletSeed(seed)
	}

	pub fn seed_file_exists(wallet_config: &WalletConfig) -> Result<(), Error> {
		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);
		if Path::new(seed_file_path).exists() {
			return Err(ErrorKind::WalletSeedExists(seed_file_path.to_owned()))?;
		}
		Ok(())
	}

	pub fn backup_seed(wallet_config: &WalletConfig) -> Result<(), Error> {
		let seed_file_name = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);

		let mut path = Path::new(seed_file_name).to_path_buf();
		path.pop();
		let mut backup_seed_file_name = format!(
			"{}{}{}.bak",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE
		);
		let mut i = 1;
		while Path::new(&backup_seed_file_name).exists() {
			backup_seed_file_name = format!(
				"{}{}{}.bak.{}",
				wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE, i
			);
			i += 1;
		}
		path.push(backup_seed_file_name.clone());
		if let Err(_) = fs::rename(seed_file_name, backup_seed_file_name.as_str()) {
			return Err(ErrorKind::GenericError(
				"Can't rename wallet seed file".to_owned(),
			))?;
		}
		warn!("{} backed up as {}", seed_file_name, backup_seed_file_name);
		Ok(())
	}

	pub fn recover_from_phrase(
		wallet_config: &WalletConfig,
		word_list: &str,
		password: &str,
	) -> Result<(), Error> {
		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);
		if WalletSeed::seed_file_exists(wallet_config).is_err() {
			WalletSeed::backup_seed(wallet_config)?;
		}
		let seed = WalletSeed::from_mnemonic(word_list)?;
		let enc_seed = EncryptedWalletSeed::from_seed(&seed, password)?;
		let enc_seed_json = serde_json::to_string_pretty(&enc_seed).context(ErrorKind::Format)?;
		let mut file = File::create(seed_file_path).context(ErrorKind::IO)?;
		file.write_all(&enc_seed_json.as_bytes())
			.context(ErrorKind::IO)?;
		warn!("Seed created from word list");
		Ok(())
	}

	pub fn show_recovery_phrase(&self) -> Result<(), Error> {
		println!("Your recovery phrase is:");
		println!();
		println!("{}", self.to_mnemonic()?);
		println!();
		println!("Please back-up these words in a non-digital format.");
		Ok(())
	}

	pub fn init_file(
		wallet_config: &WalletConfig,
		seed_length: usize,
		password: &str,
	) -> Result<WalletSeed, Error> {
		// create directory if it doesn't exist
		fs::create_dir_all(&wallet_config.data_file_dir).context(ErrorKind::IO)?;

		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);

		warn!("Generating wallet seed file at: {}", seed_file_path);
		let _ = WalletSeed::seed_file_exists(wallet_config)?;

		let seed = WalletSeed::init_new(seed_length);
		let enc_seed = EncryptedWalletSeed::from_seed(&seed, password)?;
		let enc_seed_json = serde_json::to_string_pretty(&enc_seed).context(ErrorKind::Format)?;
		let mut file = File::create(seed_file_path).context(ErrorKind::IO)?;
		file.write_all(&enc_seed_json.as_bytes())
			.context(ErrorKind::IO)?;
		seed.show_recovery_phrase()?;
		Ok(seed)
	}

	pub fn from_file(wallet_config: &WalletConfig, password: &str) -> Result<WalletSeed, Error> {
		// create directory if it doesn't exist
		fs::create_dir_all(&wallet_config.data_file_dir).context(ErrorKind::IO)?;

		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir, MAIN_SEPARATOR, SEED_FILE,
		);

		debug!("Using wallet seed file at: {}", seed_file_path);

		if Path::new(seed_file_path).exists() {
			let mut file = File::open(seed_file_path).context(ErrorKind::IO)?;
			let mut buffer = String::new();
			file.read_to_string(&mut buffer).context(ErrorKind::IO)?;
			let enc_seed: EncryptedWalletSeed =
				serde_json::from_str(&buffer).context(ErrorKind::Format)?;
			let wallet_seed = enc_seed.decrypt(password)?;
			Ok(wallet_seed)
		} else {
			error!(
				"wallet seed file {} could not be opened (grin wallet init). \
				 Run \"grin wallet init\" to initialize a new wallet.",
				seed_file_path
			);
			Err(ErrorKind::WalletSeedDoesntExist)?
		}
	}
}

/// Encrypted wallet seed, for storing on disk and decrypting
/// with provided password

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EncryptedWalletSeed {
	encrypted_seed: String,
	/// Salt, not so useful in single case but include anyhow for situations
	/// where someone wants to store many of these
	pub salt: String,
	/// Nonce
	pub nonce: String,
}

impl EncryptedWalletSeed {
	/// Create a new encrypted seed from the given seed + password
	pub fn from_seed(seed: &WalletSeed, password: &str) -> Result<EncryptedWalletSeed, Error> {
		let salt: [u8; 8] = thread_rng().gen();
		let nonce: [u8; 12] = thread_rng().gen();
		let password = password.as_bytes();
		let mut key = [0; 32];
		pbkdf2::derive(&digest::SHA512, 100, &salt, password, &mut key);
		let content = seed.0.to_vec();
		let mut enc_bytes = content.clone();
		let suffix_len = aead::CHACHA20_POLY1305.tag_len();
		for _ in 0..suffix_len {
			enc_bytes.push(0);
		}
		let sealing_key =
			aead::SealingKey::new(&aead::CHACHA20_POLY1305, &key).context(ErrorKind::Encryption)?;
		aead::seal_in_place(&sealing_key, &nonce, &[], &mut enc_bytes, suffix_len)
			.context(ErrorKind::Encryption)?;
		Ok(EncryptedWalletSeed {
			encrypted_seed: util::to_hex(enc_bytes.to_vec()),
			salt: util::to_hex(salt.to_vec()),
			nonce: util::to_hex(nonce.to_vec()),
		})
	}

	/// Decrypt seed
	pub fn decrypt(&self, password: &str) -> Result<WalletSeed, Error> {
		let mut encrypted_seed = match util::from_hex(self.encrypted_seed.clone()) {
			Ok(s) => s,
			Err(_) => return Err(ErrorKind::Encryption)?,
		};
		let salt = match util::from_hex(self.salt.clone()) {
			Ok(s) => s,
			Err(_) => return Err(ErrorKind::Encryption)?,
		};
		let nonce = match util::from_hex(self.nonce.clone()) {
			Ok(s) => s,
			Err(_) => return Err(ErrorKind::Encryption)?,
		};
		let password = password.as_bytes();
		let mut key = [0; 32];
		pbkdf2::derive(&digest::SHA512, 100, &salt, password, &mut key);

		let opening_key =
			aead::OpeningKey::new(&aead::CHACHA20_POLY1305, &key).context(ErrorKind::Encryption)?;
		let decrypted_data = aead::open_in_place(&opening_key, &nonce, &[], 0, &mut encrypted_seed)
			.context(ErrorKind::Encryption)?;

		Ok(WalletSeed::from_bytes(&decrypted_data))
	}
}
