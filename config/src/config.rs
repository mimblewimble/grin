// Copyright 2017 The Grin Developers
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

use std::env;
use std::io::Read;
use std::path::PathBuf;
use std::fs::File;

use toml;
use grin::ServerConfig;
use pow::types::MinerConfig;
use util::LoggingConfig;
use types::{ConfigError, ConfigMembers, GlobalConfig};

/// The default file name to use when trying to derive
/// the config file location

const CONFIG_FILE_NAME: &'static str = "grin.toml";
const GRIN_HOME: &'static str = ".grin";

/// Returns the defaults, as strewn throughout the code

impl Default for ConfigMembers {
	fn default() -> ConfigMembers {
		ConfigMembers {
			server: ServerConfig::default(),
			mining: Some(MinerConfig::default()),
			logging: Some(LoggingConfig::default()),
		}
	}
}

impl Default for GlobalConfig {
	fn default() -> GlobalConfig {
		GlobalConfig {
			config_file_path: None,
			using_config_file: false,
			members: Some(ConfigMembers::default()),
		}
	}
}

impl GlobalConfig {
	/// Need to decide on rules where to read the config file from,
	/// but will take a stab at logic for now

	fn derive_config_location(&mut self) -> Result<(), ConfigError> {
		// First, check working directory
		let mut config_path = env::current_dir().unwrap();
		config_path.push(CONFIG_FILE_NAME);
		if config_path.exists() {
			self.config_file_path = Some(config_path);
			return Ok(());
		}
		// Next, look in directory of executable
		let mut config_path = env::current_exe().unwrap();
		config_path.pop();
		config_path.push(CONFIG_FILE_NAME);
		if config_path.exists() {
			self.config_file_path = Some(config_path);
			return Ok(());
		}
		// Then look in {user_home}/.grin
		let config_path = env::home_dir();
		if let Some(mut p) = config_path {
			p.push(GRIN_HOME);
			p.push(CONFIG_FILE_NAME);
			if p.exists() {
				self.config_file_path = Some(p);
				return Ok(());
			}
		}

		// Give up
		Err(ConfigError::FileNotFoundError(String::from("")))
	}

	/// Takes the path to a config file, or if NONE, tries
	/// to determine a config file based on rules in
	/// derive_config_location

	pub fn new(file_path: Option<&str>) -> Result<GlobalConfig, ConfigError> {
		let mut return_value = GlobalConfig::default();
		if let Some(fp) = file_path {
			return_value.config_file_path = Some(PathBuf::from(&fp));
		} else {
			let _result = return_value.derive_config_location();
		}

		// No attempt at a config file, just return defaults
		if let None = return_value.config_file_path {
			return Ok(return_value);
		}

		// Config file path is given but not valid
		if !return_value.config_file_path.as_mut().unwrap().exists() {
			return Err(ConfigError::FileNotFoundError(String::from(
				return_value
					.config_file_path
					.as_mut()
					.unwrap()
					.to_str()
					.unwrap()
					.clone(),
			)));
		}

		// Try to parse the config file if it exists
  // explode if it does exist but something's wrong
  // with it
		return_value.read_config()
	}

	/// Read config
	pub fn read_config(mut self) -> Result<GlobalConfig, ConfigError> {
		let mut file = File::open(self.config_file_path.as_mut().unwrap())?;
		let mut contents = String::new();
		file.read_to_string(&mut contents)?;
		let decoded: Result<ConfigMembers, toml::de::Error> = toml::from_str(&contents);
		match decoded {
			Ok(mut gc) => {
				// Put the struct back together, because the config
	// file was flattened a bit
				gc.server.mining_config = gc.mining.clone();
				self.using_config_file = true;
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

	/// Serialize config
	pub fn ser_config(&mut self) -> Result<String, ConfigError> {
		let encoded: Result<String, toml::ser::Error> =
			toml::to_string(self.members.as_mut().unwrap());
		match encoded {
			Ok(enc) => return Ok(enc),
			Err(e) => {
				return Err(ConfigError::SerializationError(
					String::from(format!("{}", e)),
				));
			}
		}
	}

	/*pub fn wallet_enabled(&mut self) -> bool {
        return self.members.as_mut().unwrap().wallet.as_mut().unwrap().enable_wallet;
    }*/

	/// Enable mining
	pub fn mining_enabled(&mut self) -> bool {
		return self.members
			.as_mut()
			.unwrap()
			.mining
			.as_mut()
			.unwrap()
			.enable_mining;
	}
}

#[test]
fn test_read_config() {
	let toml_str = r#"
        #Section is optional, if not here or enable_server is false, will only run wallet
        [server]
        enable_server = true
        api_http_addr = "127.0.0.1"
        db_root = "."
        seeding_type = "None"
        test_mode = false
        #7 = FULL_NODE, not sure how to serialise this properly to use constants
        capabilities = [7]

        [server.p2p_config]
        host = "127.0.0.1"
        port = 13414

        #Mining section is optional, if it's not here it will default to not mining
        [mining]
        enable_mining = true
        wallet_listener_url = "http://127.0.0.1:13415"
        burn_reward = false
        #testing value, optional
        #slow_down_in_millis = 30

    "#;

	let mut decoded: GlobalConfig = toml::from_str(toml_str).unwrap();
	decoded.server.as_mut().unwrap().mining_config = decoded.mining;
	println!("Decoded.server: {:?}", decoded.server);
	println!("Decoded wallet: {:?}", decoded.wallet);
	panic!("panic");
}
