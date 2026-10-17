// Copyright 2026 The Grin Developers
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

//! Tor Configuration + Onion (Hidden) Service operations
use crate::util::secp::key::SecretKey;

use crate::tor::ov3::OnionV3Address;
use crate::tor::process::types::Error;
use crate::tor::types::exp_sec_key_bytes;
use ed25519_dalek::hazmat::ExpandedSecretKey;
use ed25519_dalek::SigningKey as DalekSecretKey;
use ed25519_dalek::VerifyingKey as DalekPublicKey;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, MAIN_SEPARATOR};
use std::string::String;

const SEC_KEY_FILE: &str = "hs_ed25519_secret_key";
const PUB_KEY_FILE: &str = "hs_ed25519_public_key";
const HOSTNAME_FILE: &str = "hostname";
const TORRC_FILE: &str = "torrc";
const TOR_DATA_DIR: &str = "data";
const AUTH_CLIENTS_DIR: &str = "authorized_clients";
const HIDDEN_SERVICES_DIR: &str = "onion_service_addresses";

#[cfg(unix)]
fn set_permissions(file_path: &str) -> Result<(), Error> {
	use std::os::unix::prelude::*;
	fs::set_permissions(file_path, fs::Permissions::from_mode(0o700)).map_err(Error::IO)?;
	Ok(())
}

#[cfg(windows)]
fn set_permissions(_file_path: &str) -> Result<(), Error> {
	Ok(())
}

struct TorRcConfigItem {
	pub name: String,
	pub value: String,
}

impl TorRcConfigItem {
	/// Create new
	pub fn new(name: &str, value: &str) -> Self {
		Self {
			name: name.into(),
			value: value.into(),
		}
	}
}

struct TorRcConfig {
	pub items: Vec<TorRcConfigItem>,
}

impl TorRcConfig {
	/// Create new
	pub fn new() -> Self {
		Self { items: vec![] }
	}

	/// add item
	pub fn add_item(&mut self, name: &str, value: &str) {
		self.items.push(TorRcConfigItem::new(name, value));
	}

	/// write to file
	pub fn write_to_file(&self, file_path: &str) -> Result<(), Error> {
		let mut file = File::create(file_path).map_err(Error::IO)?;
		for item in &self.items {
			file.write_all(item.name.as_bytes()).map_err(Error::IO)?;
			file.write_all(b" ").map_err(Error::IO)?;
			file.write_all(item.value.as_bytes()).map_err(Error::IO)?;
			file.write_all(b"\n").map_err(Error::IO)?;
		}
		Ok(())
	}
}

pub fn create_onion_service_sec_key_file(
	os_directory: &str,
	sec_key: &DalekSecretKey,
) -> Result<(), Error> {
	let key_file_path = &format!("{}{}{}", os_directory, MAIN_SEPARATOR, SEC_KEY_FILE);
	let mut file = File::create(key_file_path).map_err(Error::IO)?;
	// Tag is always 32 bytes, so pad with null zeroes
	file.write(b"== ed25519v1-secret: type0 ==\0\0\0")
		.map_err(Error::IO)?;
	let expanded_skey: ExpandedSecretKey = ExpandedSecretKey::from(sec_key.as_bytes());
	let sk_bytes = exp_sec_key_bytes(expanded_skey);
	file.write_all(&sk_bytes).map_err(Error::IO)?;
	Ok(())
}

pub fn create_onion_service_pub_key_file(
	os_directory: &str,
	pub_key: &DalekPublicKey,
) -> Result<(), Error> {
	let key_file_path = &format!("{}{}{}", os_directory, MAIN_SEPARATOR, PUB_KEY_FILE);
	let mut file = File::create(key_file_path).map_err(Error::IO)?;
	// Tag is always 32 bytes, so pad with null zeroes
	file.write(b"== ed25519v1-public: type0 ==\0\0\0")
		.map_err(Error::IO)?;
	file.write_all(pub_key.as_bytes()).map_err(Error::IO)?;
	Ok(())
}

pub fn create_onion_service_hostname_file(os_directory: &str, hostname: &str) -> Result<(), Error> {
	let file_path = &format!("{}{}{}", os_directory, MAIN_SEPARATOR, HOSTNAME_FILE);
	let mut file = File::create(file_path).map_err(Error::IO)?;
	file.write_all(format!("{}.onion\n", hostname).as_bytes())
		.map_err(Error::IO)?;
	Ok(())
}

pub fn create_onion_auth_clients_dir(os_directory: &str) -> Result<(), Error> {
	let auth_dir_path = &format!("{}{}{}", os_directory, MAIN_SEPARATOR, AUTH_CLIENTS_DIR);
	fs::create_dir_all(auth_dir_path).map_err(Error::IO)?;
	Ok(())
}
/// output an onion service config for the secret key, and return the address
pub fn output_onion_service_config(
	tor_config_directory: &str,
	sec_key: &SecretKey,
) -> Result<OnionV3Address, Error> {
	let d_sec_key = DalekSecretKey::from_bytes(&sec_key.0);
	let address = OnionV3Address::from_private(&sec_key.0).map_err(Error::OnionError)?;
	let hs_dir_file_path = format!(
		"{}{}{}{}{}",
		tor_config_directory, MAIN_SEPARATOR, HIDDEN_SERVICES_DIR, MAIN_SEPARATOR, address
	);

	// If file already exists, don't overwrite it, just return address
	if Path::new(&hs_dir_file_path).exists() {
		return Ok(address);
	}

	// create directory if it doesn't exist
	fs::create_dir_all(&hs_dir_file_path).map_err(Error::IO)?;

	create_onion_service_sec_key_file(&hs_dir_file_path, &d_sec_key)?;
	create_onion_service_pub_key_file(
		&hs_dir_file_path,
		&address.to_ed25519().map_err(Error::OnionError)?,
	)?;
	create_onion_service_hostname_file(&hs_dir_file_path, &address.to_string())?;
	create_onion_auth_clients_dir(&hs_dir_file_path)?;

	set_permissions(&hs_dir_file_path)?;

	Ok(address)
}

/// output torrc file given a list of hidden service directories
pub fn output_torrc(
	tor_config_directory: &str,
	wallet_listener_addr: &str,
	socks_port: &str,
	service_dirs: &[String],
	hm_tor_bridge: HashMap<String, String>,
	hm_tor_proxy: HashMap<String, String>,
) -> Result<(), Error> {
	let torrc_file_path = format!("{}{}{}", tor_config_directory, MAIN_SEPARATOR, TORRC_FILE);

	let tor_data_dir = format!("./{}", TOR_DATA_DIR);

	let mut props = TorRcConfig::new();
	props.add_item("SocksPort", socks_port);
	props.add_item("DataDirectory", &tor_data_dir);

	for dir in service_dirs {
		let service_file_name = format!("./{}{}{}", HIDDEN_SERVICES_DIR, MAIN_SEPARATOR, dir);
		props.add_item("HiddenServiceDir", &service_file_name);
		props.add_item("HiddenServicePort", &format!("80 {}", wallet_listener_addr));
	}

	if !hm_tor_bridge.is_empty() {
		props.add_item("UseBridges", "1");
		for (key, value) in hm_tor_bridge {
			props.add_item(&key, &value);
		}
	}

	if !hm_tor_proxy.is_empty() {
		for (key, value) in hm_tor_proxy {
			props.add_item(&key, &value);
		}
	}

	props.write_to_file(&torrc_file_path)?;

	Ok(())
}

/// output entire tor config for a list of secret keys
pub fn output_tor_listener_config(
	tor_config_directory: &str,
	wallet_listener_addr: &str,
	listener_keys: &[SecretKey],
	hm_tor_bridge: HashMap<String, String>,
	hm_tor_proxy: HashMap<String, String>,
) -> Result<(), Error> {
	let tor_data_dir = format!("{}{}{}", tor_config_directory, MAIN_SEPARATOR, TOR_DATA_DIR);

	// create data directory if it doesn't exist
	fs::create_dir_all(&tor_data_dir).map_err(Error::IO)?;

	let mut service_dirs = vec![];

	for k in listener_keys {
		let service_dir = output_onion_service_config(tor_config_directory, k)?;
		service_dirs.push(service_dir.to_string());
	}

	// hidden service listener doesn't need a socks port
	output_torrc(
		tor_config_directory,
		wallet_listener_addr,
		"0",
		&service_dirs,
		hm_tor_bridge,
		hm_tor_proxy,
	)?;

	Ok(())
}

/// output tor config for a sender
pub fn output_tor_sender_config(
	tor_config_dir: &str,
	socks_listener_addr: &str,
	hm_tor_bridge: HashMap<String, String>,
	hm_tor_proxy: HashMap<String, String>,
) -> Result<(), Error> {
	// create data directory if it doesn't exist
	fs::create_dir_all(tor_config_dir).map_err(Error::IO)?;

	output_torrc(
		tor_config_dir,
		"",
		socks_listener_addr,
		&[],
		hm_tor_bridge,
		hm_tor_proxy,
	)?;

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	use rand::rngs::mock::StepRng;

	use crate::util::{self, secp, static_secp_instance};

	pub fn clean_output_dir(test_dir: &str) {
		let _ = fs::remove_dir_all(test_dir);
	}

	pub fn setup(test_dir: &str) {
		util::init_test_logger();
		clean_output_dir(test_dir);
	}

	#[test]
	fn test_service_config() -> Result<(), Error> {
		let test_dir = "target/test_output/onion_service";
		setup(test_dir);
		let secp_inst = static_secp_instance();
		let secp = secp_inst.lock();
		let mut test_rng = StepRng::new(1_234_567_890_u64, 1);
		let sec_key = secp::key::SecretKey::new(&secp, &mut test_rng);
		output_onion_service_config(test_dir, &sec_key)?;
		clean_output_dir(test_dir);
		Ok(())
	}

	#[test]
	fn test_output_tor_config() -> Result<(), Error> {
		let test_dir = "./target/test_output/tor";
		setup(test_dir);
		let secp_inst = static_secp_instance();
		let secp = secp_inst.lock();
		let mut test_rng = StepRng::new(1_234_567_890_u64, 1);
		let sec_key = secp::key::SecretKey::new(&secp, &mut test_rng);
		let hm = HashMap::new();
		output_tor_listener_config(test_dir, "127.0.0.1:3415", &[sec_key], hm.clone(), hm)?;
		clean_output_dir(test_dir);
		Ok(())
	}
}
