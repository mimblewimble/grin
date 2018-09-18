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

//! Configuration file management

use dirs;
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::Read;
use std::path::PathBuf;
use toml;

use comments::insert_comments;
use servers::ServerConfig;
use types::{
	ConfigError, ConfigMembers, GlobalConfig, GlobalWalletConfig, GlobalWalletConfigMembers,
};
use util::LoggingConfig;
use wallet::WalletConfig;

/// The default file name to use when trying to derive
/// the node config file location
pub const SERVER_CONFIG_FILE_NAME: &'static str = "grin-server.toml";
/// And a wallet configuration file name
pub const WALLET_CONFIG_FILE_NAME: &'static str = "grin-wallet.toml";
const SERVER_LOG_FILE_NAME: &'static str = "grin-server.log";
const WALLET_LOG_FILE_NAME: &'static str = "grin-wallet.log";
const GRIN_HOME: &'static str = ".grin";
const GRIN_CHAIN_DIR: &'static str = "chain_data";
const GRIN_WALLET_DIR: &'static str = "wallet_data";

fn get_grin_path() -> Result<PathBuf, ConfigError> {
	// Check if grin dir exists
	let grin_path = {
		match dirs::home_dir() {
			Some(mut p) => {
				p.push(GRIN_HOME);
				p
			}
			None => {
				let mut pb = PathBuf::new();
				pb.push(GRIN_HOME);
				pb
			}
		}
	};
	// Create if the default path doesn't exist
	if !grin_path.exists() {
		fs::create_dir_all(grin_path.clone())?;
	}
	Ok(grin_path)
}

fn check_config_current_dir(path: &str) -> Option<PathBuf> {
	let p = env::current_dir();
	let mut c = match p {
		Ok(c) => c,
		Err(_) => {
			return None;
		}
	};
	c.push(path);
	if c.exists() {
		return Some(c);
	}
	None
}

/// Handles setup and detection of paths for node
pub fn initial_setup_server() -> Result<GlobalConfig, ConfigError> {
	// Use config file if current directory if it exists, .grin home otherwise
	if let Some(p) = check_config_current_dir(SERVER_CONFIG_FILE_NAME) {
		GlobalConfig::new(p.to_str().unwrap())
	} else {
		// Check if grin dir exists
		let grin_path = get_grin_path()?;

		// Get path to default config file
		let mut config_path = grin_path.clone();
		config_path.push(SERVER_CONFIG_FILE_NAME);

		// Spit it out if it doesn't exist
		if !config_path.exists() {
			let mut default_config = GlobalConfig::default();
			// update paths relative to current dir
			default_config.update_paths(&grin_path);
			default_config.write_to_file(config_path.to_str().unwrap())?;
		}
		GlobalConfig::new(config_path.to_str().unwrap())
	}
}

/// Handles setup and detection of paths for wallet
pub fn initial_setup_wallet() -> Result<GlobalWalletConfig, ConfigError> {
	// Use config file if current directory if it exists, .grin home otherwise
	if let Some(p) = check_config_current_dir(WALLET_CONFIG_FILE_NAME) {
		GlobalWalletConfig::new(p.to_str().unwrap())
	} else {
		// Check if grin dir exists
		let grin_path = get_grin_path()?;

		// Get path to default config file
		let mut config_path = grin_path.clone();
		config_path.push(WALLET_CONFIG_FILE_NAME);

		// Spit it out if it doesn't exist
		if !config_path.exists() {
			let mut default_config = GlobalWalletConfig::default();
			// update paths relative to current dir
			default_config.update_paths(&grin_path);
			default_config.write_to_file(config_path.to_str().unwrap())?;
		}
		GlobalWalletConfig::new(config_path.to_str().unwrap())
	}
}

/// Returns the defaults, as strewn throughout the code
impl Default for ConfigMembers {
	fn default() -> ConfigMembers {
		ConfigMembers {
			server: ServerConfig::default(),
			logging: Some(LoggingConfig::default()),
		}
	}
}

impl Default for GlobalConfig {
	fn default() -> GlobalConfig {
		GlobalConfig {
			config_file_path: None,
			members: Some(ConfigMembers::default()),
		}
	}
}

impl Default for GlobalWalletConfigMembers {
	fn default() -> GlobalWalletConfigMembers {
		GlobalWalletConfigMembers {
			logging: Some(LoggingConfig::default()),
			wallet: WalletConfig::default(),
		}
	}
}

impl Default for GlobalWalletConfig {
	fn default() -> GlobalWalletConfig {
		GlobalWalletConfig {
			config_file_path: None,
			members: Some(GlobalWalletConfigMembers::default()),
		}
	}
}

impl GlobalConfig {
	/// Requires the path to a config file
	pub fn new(file_path: &str) -> Result<GlobalConfig, ConfigError> {
		let mut return_value = GlobalConfig::default();
		return_value.config_file_path = Some(PathBuf::from(&file_path));

		// Config file path is given but not valid
		let config_file = return_value.config_file_path.clone().unwrap();
		if !config_file.exists() {
			return Err(ConfigError::FileNotFoundError(String::from(
				config_file.to_str().unwrap(),
			)));
		}

		// Try to parse the config file if it exists, explode if it does exist but
		// something's wrong with it
		return_value.read_config()
	}

	/// Read config
	fn read_config(mut self) -> Result<GlobalConfig, ConfigError> {
		let mut file = File::open(self.config_file_path.as_mut().unwrap())?;
		let mut contents = String::new();
		file.read_to_string(&mut contents)?;
		let decoded: Result<ConfigMembers, toml::de::Error> = toml::from_str(&contents);
		match decoded {
			Ok(mut gc) => {
				gc.server.validation_check();
				self.members = Some(gc);
				return Ok(self);
			}
			Err(e) => {
				return Err(ConfigError::ParseError(
					String::from(
						self.config_file_path
							.as_mut()
							.unwrap()
							.to_str()
							.unwrap()
							.clone(),
					),
					String::from(format!("{}", e)),
				));
			}
		}
	}

	/// Update paths
	pub fn update_paths(&mut self, grin_home: &PathBuf) {
		// need to update server chain path
		let mut chain_path = grin_home.clone();
		chain_path.push(GRIN_CHAIN_DIR);
		self.members.as_mut().unwrap().server.db_root = chain_path.to_str().unwrap().to_owned();
		let mut log_path = grin_home.clone();
		log_path.push(SERVER_LOG_FILE_NAME);
		self.members
			.as_mut()
			.unwrap()
			.logging
			.as_mut()
			.unwrap()
			.log_file_path = log_path.to_str().unwrap().to_owned();
	}

	/// Enable mining
	pub fn stratum_enabled(&mut self) -> bool {
		return self
			.members
			.as_mut()
			.unwrap()
			.server
			.stratum_mining_config
			.as_mut()
			.unwrap()
			.enable_stratum_server
			.unwrap();
	}

	/// Serialize config
	pub fn ser_config(&mut self) -> Result<String, ConfigError> {
		let encoded: Result<String, toml::ser::Error> =
			toml::to_string(self.members.as_mut().unwrap());
		match encoded {
			Ok(enc) => return Ok(enc),
			Err(e) => {
				return Err(ConfigError::SerializationError(String::from(format!(
					"{}",
					e
				))));
			}
		}
	}

	/// Write configuration to a file
	pub fn write_to_file(&mut self, name: &str) -> Result<(), ConfigError> {
		let conf_out = self.ser_config()?;
		let conf_out = insert_comments(conf_out);
		let mut file = File::create(name)?;
		file.write_all(conf_out.as_bytes())?;
		Ok(())
	}
}

/// TODO: Properly templatize these structs (if it's worth the effort)
impl GlobalWalletConfig {
	/// Requires the path to a config file
	pub fn new(file_path: &str) -> Result<GlobalWalletConfig, ConfigError> {
		let mut return_value = GlobalWalletConfig::default();
		return_value.config_file_path = Some(PathBuf::from(&file_path));

		// Config file path is given but not valid
		let config_file = return_value.config_file_path.clone().unwrap();
		if !config_file.exists() {
			return Err(ConfigError::FileNotFoundError(String::from(
				config_file.to_str().unwrap(),
			)));
		}

		// Try to parse the config file if it exists, explode if it does exist but
		// something's wrong with it
		return_value.read_config()
	}

	/// Read config
	fn read_config(mut self) -> Result<GlobalWalletConfig, ConfigError> {
		let mut file = File::open(self.config_file_path.as_mut().unwrap())?;
		let mut contents = String::new();
		file.read_to_string(&mut contents)?;
		let decoded: Result<GlobalWalletConfigMembers, toml::de::Error> = toml::from_str(&contents);
		match decoded {
			Ok(gc) => {
				self.members = Some(gc);
				return Ok(self);
			}
			Err(e) => {
				return Err(ConfigError::ParseError(
					String::from(
						self.config_file_path
							.as_mut()
							.unwrap()
							.to_str()
							.unwrap()
							.clone(),
					),
					String::from(format!("{}", e)),
				));
			}
		}
	}

	/// Update paths
	pub fn update_paths(&mut self, wallet_home: &PathBuf) {
		let mut wallet_path = wallet_home.clone();
		wallet_path.push(GRIN_WALLET_DIR);
		self.members.as_mut().unwrap().wallet.data_file_dir =
			wallet_path.to_str().unwrap().to_owned();
		let mut log_path = wallet_home.clone();
		log_path.push(WALLET_LOG_FILE_NAME);
		self.members
			.as_mut()
			.unwrap()
			.logging
			.as_mut()
			.unwrap()
			.log_file_path = log_path.to_str().unwrap().to_owned();
	}

	/// Serialize config
	pub fn ser_config(&mut self) -> Result<String, ConfigError> {
		let encoded: Result<String, toml::ser::Error> =
			toml::to_string(self.members.as_mut().unwrap());
		match encoded {
			Ok(enc) => return Ok(enc),
			Err(e) => {
				return Err(ConfigError::SerializationError(String::from(format!(
					"{}",
					e
				))));
			}
		}
	}

	/// Write configuration to a file
	pub fn write_to_file(&mut self, name: &str) -> Result<(), ConfigError> {
		let conf_out = self.ser_config()?;
		let conf_out = insert_comments(conf_out);
		let mut file = File::create(name)?;
		file.write_all(conf_out.as_bytes())?;
		Ok(())
	}
}
