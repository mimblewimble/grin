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
use std::io::Read;
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
/// Node Owner API secret
pub const NODE_OWNER_API_SECRET_FILE_NAME: &str = ".node_owner_api_secret";
/// Node Foreign API secret
pub const NODE_FOREIGN_API_SECRET_FILE_NAME: &str = ".node_foreign_api_secret";
/// Old Node Owner API secret for forward compatibility
pub const OLD_NODE_OWNER_API_SECRET_FILE_NAME: &str = ".api_secret";
/// Old Node Foreign API secret for forward compatibility
pub const OLD_NODE_FOREIGN_API_SECRET_FILE_NAME: &str = ".foreign_api_secret";

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

/// Check that the default/custom api secret file exists and is valid when the config file exist
fn check_api_file_existing_config(api_secret: String) -> Result<(), ConfigError> {
	let mut api_secret_path = PathBuf::new();
	api_secret_path.push(api_secret);
	if !api_secret_path.exists() {
		init_api_secret(&api_secret_path)
	} else {
		check_api_secret(&api_secret_path)
	}
}

/// Check that the api secret file exists and is valid when the config file does not exist
fn check_api_secret_files(
	chain_type: &global::ChainTypes,
	secret_file_name: &str,
	old_secret_file_name: &str,
) -> Result<(), ConfigError> {
	let grin_path = get_grin_path(chain_type)?;
	let mut api_secret_path = grin_path.clone();
	api_secret_path.push(secret_file_name);
	let mut old_api_secret_path = grin_path.clone();
	old_api_secret_path.push(old_secret_file_name);
	if api_secret_path.exists() {
		check_api_secret(&api_secret_path)
	} else if old_api_secret_path.exists() {
		check_api_secret(&old_api_secret_path)
	} else {
		init_api_secret(&api_secret_path)
	}
}

/// Handles setup and detection of paths for node
pub fn initial_setup_server(chain_type: &global::ChainTypes) -> Result<GlobalConfig, ConfigError> {
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
			check_api_secret_files(
				chain_type,
				NODE_OWNER_API_SECRET_FILE_NAME,
				OLD_NODE_OWNER_API_SECRET_FILE_NAME,
			)?;
			check_api_secret_files(
				chain_type,
				NODE_FOREIGN_API_SECRET_FILE_NAME,
				OLD_NODE_FOREIGN_API_SECRET_FILE_NAME,
			)?;
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
		let mut file = File::open(self.config_file_path.as_mut().unwrap())?;
		let mut contents = String::new();
		file.read_to_string(&mut contents)?;
		let fixed = GlobalConfig::forwards_compatibility(contents);
		let decoded: Result<ConfigMembers, toml::de::Error> = toml::from_str(&fixed);
		match decoded {
			Ok(gc) => {
				self.members = Some(gc);
				if let Some(p) = self
					.members
					.as_mut()
					.unwrap()
					.server
					.node_owner_api_secret_path
					.clone()
				{
					check_api_file_existing_config(p)?;
				}
				if let Some(p) = self
					.members
					.as_mut()
					.unwrap()
					.server
					.node_foreign_api_secret_path
					.clone()
				{
					check_api_file_existing_config(p)?;
				}
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
		let mut node_owner_api_secret_path = grin_home.clone();
		node_owner_api_secret_path.push(OLD_NODE_OWNER_API_SECRET_FILE_NAME);
		if !node_owner_api_secret_path.exists() {
			node_owner_api_secret_path.pop();
			node_owner_api_secret_path.push(NODE_OWNER_API_SECRET_FILE_NAME);
		}
		self.members
			.as_mut()
			.unwrap()
			.server
			.node_owner_api_secret_path = Some(node_owner_api_secret_path.to_str().unwrap().to_owned());
		let mut node_foreign_api_secret_path = grin_home.clone();
		node_foreign_api_secret_path.push(OLD_NODE_FOREIGN_API_SECRET_FILE_NAME);
		if !node_foreign_api_secret_path.exists() {
			node_foreign_api_secret_path.pop();
			node_foreign_api_secret_path.push(NODE_FOREIGN_API_SECRET_FILE_NAME);
		}
		self.members
			.as_mut()
			.unwrap()
			.server
			.node_foreign_api_secret_path = Some(node_foreign_api_secret_path.to_str().unwrap().to_owned());
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

	// For forwards compatibility of old config
	fn forwards_compatibility(conf: String) -> String {
		// Needs `Warning` log level changed to standard log::Level `WARN`
		conf.replace("Warning", "WARN")
			// Needs `api_secret_path` toml key compatibility to the actual "node_owner_api_secret_path" field
			.replace("\napi_secret_path", "\nnode_owner_api_secret_path")
			// Needs `foreign_api_secret_path` toml key compatibility to the actual "node_foreign_api_secret_path" field
			.replace(
				"\nforeign_api_secret_path",
				"\nnode_foreign_api_secret_path",
			)
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
fn test_forwards_compatibility() {
	let config = "Warning \nforeign_api_secret_path \napi_secret_path".to_string();
	let fixed_config = GlobalConfig::forwards_compatibility(config);
	assert_eq!(
		fixed_config,
		"WARN \nnode_foreign_api_secret_path \nnode_owner_api_secret_path"
	);
}
