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
use std::fs::File;
use std::io::prelude::*;
use std::io::Read;
use std::path::PathBuf;
use toml;

use comments::insert_comments;
use servers::ServerConfig;
use types::{ConfigError, ConfigMembers, GlobalConfig};
use util::LoggingConfig;
use wallet::WalletConfig;

/// The default file name to use when trying to derive
/// the config file location

const CONFIG_FILE_NAME: &'static str = "grin.toml";
const GRIN_HOME: &'static str = ".grin";

/// Returns the defaults, as strewn throughout the code

impl Default for ConfigMembers {
	fn default() -> ConfigMembers {
		ConfigMembers {
			server: ServerConfig::default(),
			logging: Some(LoggingConfig::default()),
			wallet: WalletConfig::default(),
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
		let config_path = dirs::home_dir();
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

	/// Takes the path to a config file, or if NONE, tries to determine a config
	/// file based on rules in derive_config_location
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
