// Copyright 2021 The Grin Developers
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

use rand::distributions::{Alphanumeric, Distribution};
use rand::thread_rng;
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::BufReader;
use std::path::PathBuf;

use crate::comments::insert_comments;
use crate::core::global;
use crate::p2p;
use crate::servers::ServerConfig;
use crate::types::{ConfigError, ConfigMembers, GlobalConfig};
use crate::util::logger::LoggingConfig;

/// The default file name to use when trying to derive
/// the node config file location
pub const SERVER_CONFIG_FILE_NAME: &str = "grin-server.toml";
const SERVER_LOG_FILE_NAME: &str = "grin-server.log";
const GRIN_HOME: &str = ".grin";
const GRIN_CHAIN_DIR: &str = "chain_data";
/// Node Rest API and V2 Owner API secret
pub const API_SECRET_FILE_NAME: &str = ".api_secret";
/// Foreign API secret
pub const FOREIGN_API_SECRET_FILE_NAME: &str = ".foreign_api_secret";

fn get_grin_path(chain_type: &global::ChainTypes) -> Result<PathBuf, ConfigError> {
	// Check if grin dir exists
	let mut grin_path = match dirs::home_dir() {
		Some(p) => p,
		None => PathBuf::new(),
	};
	grin_path.push(GRIN_HOME);
	grin_path.push(chain_type.shortname());
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

/// Create file with api secret
pub fn init_api_secret(api_secret_path: &PathBuf) -> Result<(), ConfigError> {
	let mut api_secret_file = File::create(api_secret_path)?;
	let api_secret: String = Alphanumeric
		.sample_iter(&mut thread_rng())
		.take(20)
		.collect();
	api_secret_file.write_all(api_secret.as_bytes())?;
	Ok(())
}

/// Check if file contains a secret and nothing else
pub fn check_api_secret(api_secret_path: &PathBuf) -> Result<(), ConfigError> {
	let api_secret_file = File::open(api_secret_path)?;
	let buf_reader = BufReader::new(api_secret_file);
	let mut lines_iter = buf_reader.lines();
	let first_line = lines_iter.next();
	if first_line.is_none() || first_line.unwrap().is_err() {
		fs::remove_file(api_secret_path)?;
		init_api_secret(api_secret_path)?;
	}
	Ok(())
}

/// Check that the api secret files exist and are valid
fn check_api_secret_files(
	chain_type: &global::ChainTypes,
	secret_file_name: &str,
) -> Result<(), ConfigError> {
	let grin_path = get_grin_path(chain_type)?;
	let mut api_secret_path = grin_path;
	api_secret_path.push(secret_file_name);
	if !api_secret_path.exists() {
		init_api_secret(&api_secret_path)
	} else {
		check_api_secret(&api_secret_path)
	}
}

/// Handles setup and detection of paths for node
pub fn initial_setup_server(chain_type: &global::ChainTypes) -> Result<GlobalConfig, ConfigError> {
	check_api_secret_files(chain_type, API_SECRET_FILE_NAME)?;
	check_api_secret_files(chain_type, FOREIGN_API_SECRET_FILE_NAME)?;
	// Use config file if current directory if it exists, .grin home otherwise
	if let Some(p) = check_config_current_dir(SERVER_CONFIG_FILE_NAME) {
		GlobalConfig::new(p.to_str().unwrap())
	} else {
		// Check if grin dir exists
		let grin_path = get_grin_path(chain_type)?;

		// Get path to default config file
		let mut config_path = grin_path.clone();
		config_path.push(SERVER_CONFIG_FILE_NAME);

		// Spit it out if it doesn't exist
		if !config_path.exists() {
			let mut default_config = GlobalConfig::for_chain(chain_type);
			// update paths relative to current dir
			default_config.update_paths(&grin_path);
			default_config.write_to_file(config_path.to_str().unwrap())?;
		}

		GlobalConfig::new(config_path.to_str().unwrap())
	}
}

/// Returns the defaults, as strewn throughout the code
impl Default for ConfigMembers {
	fn default() -> ConfigMembers {
		ConfigMembers {
			config_file_version: Some(2),
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

impl GlobalConfig {
	/// Same as GlobalConfig::default() but further tweaks parameters to
	/// apply defaults for each chain type
	pub fn for_chain(chain_type: &global::ChainTypes) -> GlobalConfig {
		let mut defaults_conf = GlobalConfig::default();
		let mut defaults = &mut defaults_conf.members.as_mut().unwrap().server;
		defaults.chain_type = chain_type.clone();

		match *chain_type {
			global::ChainTypes::Mainnet => {}
			global::ChainTypes::Testnet => {
				defaults.api_http_addr = "127.0.0.1:13413".to_owned();
				defaults.p2p_config.port = 13414;
				defaults
					.stratum_mining_config
					.as_mut()
					.unwrap()
					.stratum_server_addr = Some("127.0.0.1:13416".to_owned());
				defaults
					.stratum_mining_config
					.as_mut()
					.unwrap()
					.wallet_listener_url = "http://127.0.0.1:13415".to_owned();
			}
			global::ChainTypes::UserTesting => {
				defaults.api_http_addr = "127.0.0.1:23413".to_owned();
				defaults.p2p_config.port = 23414;
				defaults.p2p_config.seeding_type = p2p::Seeding::None;
				defaults
					.stratum_mining_config
					.as_mut()
					.unwrap()
					.stratum_server_addr = Some("127.0.0.1:23416".to_owned());
				defaults
					.stratum_mining_config
					.as_mut()
					.unwrap()
					.wallet_listener_url = "http://127.0.0.1:23415".to_owned();
			}
			global::ChainTypes::AutomatedTesting => {
				panic!("Can't run automated testing directly");
			}
		}
		defaults_conf
	}

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
		let config_file_path = self.config_file_path.as_ref().unwrap();
		let contents = fs::read_to_string(config_file_path)?;
		let migrated = GlobalConfig::migrate_config_file_version_none_to_2(contents.clone());
		if contents != migrated {
			fs::write(config_file_path, &migrated)?;
		}

		let fixed = GlobalConfig::fix_warning_level(migrated);
		let decoded: Result<ConfigMembers, toml::de::Error> = toml::from_str(&fixed);
		match decoded {
			Ok(gc) => {
				self.members = Some(gc);
				return Ok(self);
			}
			Err(e) => {
				return Err(ConfigError::ParseError(
					self.config_file_path.unwrap().to_str().unwrap().to_string(),
					format!("{}", e),
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
		let mut api_secret_path = grin_home.clone();
		api_secret_path.push(API_SECRET_FILE_NAME);
		self.members.as_mut().unwrap().server.api_secret_path =
			Some(api_secret_path.to_str().unwrap().to_owned());
		let mut foreign_api_secret_path = grin_home.clone();
		foreign_api_secret_path.push(FOREIGN_API_SECRET_FILE_NAME);
		self.members
			.as_mut()
			.unwrap()
			.server
			.foreign_api_secret_path = Some(foreign_api_secret_path.to_str().unwrap().to_owned());
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
				return Err(ConfigError::SerializationError(format!("{}", e)));
			}
		}
	}

	/// Write configuration to a file
	pub fn write_to_file(&mut self, name: &str) -> Result<(), ConfigError> {
		let conf_out = self.ser_config()?;
		let fixed_config = GlobalConfig::fix_log_level(conf_out);
		let commented_config = insert_comments(fixed_config);
		let mut file = File::create(name)?;
		file.write_all(commented_config.as_bytes())?;
		Ok(())
	}

	/// This migration does the following:
	/// - Adds "config_file_version = 2"
	/// - If server.pool_config.accept_fee_base is 1000000, change it to 500000
	/// - Remove "#a setting to 1000000 will be overridden to 500000 to respect the fixfees RFC"
	fn migrate_config_file_version_none_to_2(config_str: String) -> String {
		// Parse existing config and return unchanged if not eligible for migration

		let mut config: ConfigMembers =
			toml::from_str(&GlobalConfig::fix_warning_level(config_str.clone())).unwrap();
		if config.config_file_version != None {
			return config_str;
		}

		// Apply changes both textually and structurally

		let config_str = config_str.replace("\n#########################################\n### SERVER CONFIGURATION              ###", "\nconfig_file_version = 2\n\n#########################################\n### SERVER CONFIGURATION              ###");
		config.config_file_version = Some(2);

		let config_str = config_str.replace(
			"\naccept_fee_base = 1000000\n",
			"\naccept_fee_base = 500000\n",
		);
		if config.server.pool_config.accept_fee_base == 1000000 {
			config.server.pool_config.accept_fee_base = 500000;
		}

		let config_str = config_str.replace(
			"\n#a setting to 1000000 will be overridden to 500000 to respect the fixfees RFC\n",
			"\n",
		);

		// Verify equivalence

		assert_eq!(
			config,
			toml::from_str(&GlobalConfig::fix_warning_level(config_str.clone())).unwrap()
		);

		config_str
	}

	// For forwards compatibility old config needs `Warning` log level changed to standard log::Level `WARN`
	fn fix_warning_level(conf: String) -> String {
		conf.replace("Warning", "WARN")
	}

	// For backwards compatibility only first letter of log level should be capitalised.
	fn fix_log_level(conf: String) -> String {
		conf.replace("TRACE", "Trace")
			.replace("DEBUG", "Debug")
			.replace("INFO", "Info")
			.replace("WARN", "Warning")
			.replace("ERROR", "Error")
	}
}

#[test]
fn test_fix_log_level() {
	let config = "TRACE DEBUG INFO WARN ERROR".to_string();
	let fixed_config = GlobalConfig::fix_log_level(config);
	assert_eq!(fixed_config, "Trace Debug Info Warning Error");
}

#[test]
fn test_fix_warning_level() {
	let config = "Warning".to_string();
	let fixed_config = GlobalConfig::fix_warning_level(config);
	assert_eq!(fixed_config, "WARN");
}
