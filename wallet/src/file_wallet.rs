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

use blake2;
use rand::{thread_rng, Rng};
use std::cmp::min;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::MAIN_SEPARATOR;
use std::path::Path;

use serde_json;
use tokio_core::reactor;
use tokio_retry::Retry;
use tokio_retry::strategy::FibonacciBackoff;

use failure::{Fail, ResultExt};

use keychain::{self, Keychain};
use util;
use util::LOGGER;

use libwallet::types::*;

const DAT_FILE: &'static str = "wallet.dat";
const BCK_FILE: &'static str = "wallet.bck";
const LOCK_FILE: &'static str = "wallet.lock";
const SEED_FILE: &'static str = "wallet.seed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
	// Right now the decision to run or not a wallet is based on the command.
	// This may change in the near-future.
	// pub enable_wallet: bool,

	// The api interface/ip_address that this api server (i.e. this wallet) will run
	// by default this is 127.0.0.1 (and will not accept connections from external clients)
	pub api_listen_interface: String,
	// The port this wallet will run on
	pub api_listen_port: u16,
	// The api address of a running server node against which transaction inputs
	// will be checked during send
	pub check_node_api_http_addr: String,
	// The directory in which wallet files are stored
	pub data_file_dir: String,
}

impl Default for WalletConfig {
	fn default() -> WalletConfig {
		WalletConfig {
			// enable_wallet: false,
			api_listen_interface: "127.0.0.1".to_string(),
			api_listen_port: 13415,
			check_node_api_http_addr: "http://127.0.0.1:13413".to_string(),
			data_file_dir: ".".to_string(),
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

	fn from_hex(hex: &str) -> Result<WalletSeed, Error> {
		let bytes =
			util::from_hex(hex.to_string()).context(ErrorKind::GenericError("Invalid hex"))?;
		Ok(WalletSeed::from_bytes(&bytes))
	}

	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	pub fn derive_keychain(&self, password: &str) -> Result<keychain::Keychain, Error> {
		let seed = blake2::blake2b::blake2b(64, &password.as_bytes(), &self.0);
		let result = keychain::Keychain::from_seed(seed.as_bytes()).context(ErrorKind::Keychain)?;
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

		debug!(LOGGER, "Generating wallet seed file at: {}", seed_file_path,);

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

/// Wallet information tracking all our outputs. Based on HD derivation and
/// avoids storing any key data, only storing output amounts and child index.
#[derive(Debug, Clone)]
pub struct FileWallet {
	/// Keychain
	pub keychain: Keychain,
	/// Configuration
	pub config: WalletConfig,
	/// List of outputs
	pub outputs: HashMap<String, OutputData>,
	/// Data file path
	pub data_file_path: String,
	/// Backup file path
	pub backup_file_path: String,
	/// lock file path
	pub lock_file_path: String,
}

impl WalletBackend for FileWallet {
	/// Return the keychain being used
	fn keychain(&mut self) -> &mut Keychain {
		&mut self.keychain
	}

	/// Return URL for check node
	fn node_url(&self) -> &str {
		&self.config.check_node_api_http_addr
	}

	/// Return the outputs directly
	fn outputs(&mut self) -> &mut HashMap<String, OutputData> {
		&mut self.outputs
	}

	/// Allows for reading wallet data (without needing to acquire the write
	/// lock).
	fn read_wallet<T, F>(&mut self, f: F) -> Result<T, Error>
	where
		F: FnOnce(&mut Self) -> Result<T, Error>,
	{
		self.read_or_create_paths()?;
		f(self)
	}

	/// Allows the reading and writing of the wallet data within a file lock.
	/// Just provide a closure taking a mutable FileWallet. The lock should
	/// be held for as short a period as possible to avoid contention.
	/// Note that due to the impossibility to do an actual file lock easily
	/// across operating systems, this just creates a lock file with a "should
	/// not exist" option.
	fn with_wallet<T, F>(&mut self, f: F) -> Result<T, Error>
	where
		F: FnOnce(&mut Self) -> T,
	{
		// create directory if it doesn't exist
		fs::create_dir_all(self.config.data_file_dir.clone()).unwrap_or_else(|why| {
			info!(LOGGER, "! {:?}", why.kind());
		});

		info!(LOGGER, "Acquiring wallet lock ...");

		let lock_file_path = self.lock_file_path.clone();
		let action = || {
			trace!(LOGGER, "making lock file for wallet lock");
			fs::create_dir(&lock_file_path)
		};

		// use tokio_retry to cleanly define some retry logic
		let mut core = reactor::Core::new().unwrap();
		let retry_strategy = FibonacciBackoff::from_millis(1000).take(10);
		let retry_future = Retry::spawn(core.handle(), retry_strategy, action);
		let retry_result = core.run(retry_future);

		match retry_result {
			Ok(_) => {}
			Err(e) => {
				error!(
					LOGGER,
					"Failed to acquire wallet lock file (multiple retries)",
				);
				return Err(
					e.context(ErrorKind::FileWallet("Failed to acquire lock file"))
						.into(),
				);
			}
		}

		// We successfully acquired the lock - so do what needs to be done.
		self.read_or_create_paths()?;
		self.write(&self.backup_file_path)?;
		let res = f(self);
		self.write(&self.data_file_path)?;

		// delete the lock file
		fs::remove_dir(&self.lock_file_path).context(ErrorKind::FileWallet(
			"Could not remove wallet lock file. Maybe insufficient rights?",
		))?;

		info!(LOGGER, "... released wallet lock");

		Ok(res)
	}

	/// Append a new output data to the wallet data.
	/// TODO - we should check for overwriting here - only really valid for
	/// unconfirmed coinbase
	fn add_output(&mut self, out: OutputData) {
		self.outputs.insert(out.key_id.to_hex(), out.clone());
	}

	// TODO - careful with this, only for Unconfirmed (maybe Locked)?
	fn delete_output(&mut self, id: &keychain::Identifier) {
		self.outputs.remove(&id.to_hex());
	}

	/// Lock an output data.
	/// TODO - we should track identifier on these outputs (not just n_child)
	fn lock_output(&mut self, out: &OutputData) {
		if let Some(out_to_lock) = self.outputs.get_mut(&out.key_id.to_hex()) {
			if out_to_lock.value == out.value {
				out_to_lock.lock()
			}
		}
	}

	/// get a single output
	fn get_output(&self, key_id: &keychain::Identifier) -> Option<&OutputData> {
		self.outputs.get(&key_id.to_hex())
	}

	/// Next child index when we want to create a new output.
	fn next_child(&self, root_key_id: keychain::Identifier) -> u32 {
		let mut max_n = 0;
		for out in self.outputs.values() {
			if max_n < out.n_child && out.root_key_id == root_key_id {
				max_n = out.n_child;
			}
		}
		max_n + 1
	}

	/// Select spendable coins from the wallet.
	/// Default strategy is to spend the maximum number of outputs (up to
	/// max_outputs). Alternative strategy is to spend smallest outputs first
	/// but only as many as necessary. When we introduce additional strategies
	/// we should pass something other than a bool in.
	fn select_coins(
		&self,
		root_key_id: keychain::Identifier,
		amount: u64,
		current_height: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		select_all: bool,
	) -> Vec<OutputData> {
		// first find all eligible outputs based on number of confirmations
		let mut eligible = self.outputs
			.values()
			.filter(|out| {
				out.root_key_id == root_key_id
					&& out.eligible_to_spend(current_height, minimum_confirmations)
			})
			.cloned()
			.collect::<Vec<OutputData>>();

		// sort eligible outputs by increasing value
		eligible.sort_by_key(|out| out.value);

		// use a sliding window to identify potential sets of possible outputs to spend
		// Case of amount > total amount of max_outputs(500):
		// The limit exists because by default, we always select as many inputs as
		// possible in a transaction, to reduce both the Output set and the fees.
		// But that only makes sense up to a point, hence the limit to avoid being too
		// greedy. But if max_outputs(500) is actually not enought to cover the whole
		// amount, the wallet should allow going over it to satisfy what the user
		// wants to send. So the wallet considers max_outputs more of a soft limit.
		if eligible.len() > max_outputs {
			for window in eligible.windows(max_outputs) {
				let windowed_eligibles = window.iter().cloned().collect::<Vec<_>>();
				if let Some(outputs) = self.select_from(amount, select_all, windowed_eligibles) {
					return outputs;
				}
			}
			// Not exist in any window of which total amount >= amount.
			// Then take coins from the smallest one up to the total amount of selected
			// coins = the amount.
			if let Some(outputs) = self.select_from(amount, false, eligible.clone()) {
				debug!(
					LOGGER,
					"Extending maximum number of outputs. {} outputs selected.",
					outputs.len()
				);
				return outputs;
			}
		} else {
			if let Some(outputs) = self.select_from(amount, select_all, eligible.clone()) {
				return outputs;
			}
		}

		// we failed to find a suitable set of outputs to spend,
		// so return the largest amount we can so we can provide guidance on what is
		// possible
		eligible.reverse();
		eligible.iter().take(max_outputs).cloned().collect()
	}
}

impl FileWallet {
	/// Create a new FileWallet instance
	pub fn new(config: WalletConfig, keychain: Keychain) -> Result<Self, Error> {
		let mut retval = FileWallet {
			keychain: keychain,
			config: config.clone(),
			outputs: HashMap::new(),
			data_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, DAT_FILE),
			backup_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, BCK_FILE),
			lock_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, LOCK_FILE),
		};
		match retval.read_or_create_paths() {
			Ok(_) => Ok(retval),
			Err(e) => Err(e),
		}
	}

	/// Read the wallet data or create brand files if the data
	/// files don't yet exist
	fn read_or_create_paths(&mut self) -> Result<(), Error> {
		if !Path::new(&self.config.data_file_dir.clone()).exists() {
			fs::create_dir_all(&self.config.data_file_dir.clone()).unwrap_or_else(|why| {
				info!(LOGGER, "! {:?}", why.kind());
			});
		}
		if Path::new(&self.data_file_path.clone()).exists() {
			self.read()?;
		}
		Ok(())
	}

	/// Read output_data vec from disk.
	fn read_outputs(&self) -> Result<Vec<OutputData>, Error> {
		let data_file = File::open(self.data_file_path.clone())
			.context(ErrorKind::FileWallet(&"Could not open wallet file"))?;
		serde_json::from_reader(data_file).map_err(|e| {
			e.context(ErrorKind::FileWallet(&"Error reading wallet file "))
				.into()
		})
	}

	/// Populate wallet_data with output_data from disk.
	fn read(&mut self) -> Result<(), Error> {
		let outputs = self.read_outputs()?;
		self.outputs = HashMap::new();
		for out in outputs {
			self.add_output(out);
		}
		Ok(())
	}

	/// Write the wallet data to disk.
	fn write(&self, data_file_path: &str) -> Result<(), Error> {
		let mut data_file = File::create(data_file_path)
			.map_err(|e| e.context(ErrorKind::FileWallet(&"Could not create ")))?;
		let mut outputs = self.outputs.values().collect::<Vec<_>>();
		outputs.sort();
		let res_json = serde_json::to_vec_pretty(&outputs)
			.map_err(|e| e.context(ErrorKind::FileWallet("Error serializing wallet data")))?;
		data_file
			.write_all(res_json.as_slice())
			.context(ErrorKind::FileWallet(&"Error writing wallet file"))
			.map_err(|e| e.into())
	}

	// Select the full list of outputs if we are using the select_all strategy.
	// Otherwise select just enough outputs to cover the desired amount.
	fn select_from(
		&self,
		amount: u64,
		select_all: bool,
		outputs: Vec<OutputData>,
	) -> Option<Vec<OutputData>> {
		let total = outputs.iter().fold(0, |acc, x| acc + x.value);
		if total >= amount {
			if select_all {
				return Some(outputs.iter().cloned().collect());
			} else {
				let mut selected_amount = 0;
				return Some(
					outputs
						.iter()
						.take_while(|out| {
							let res = selected_amount < amount;
							selected_amount += out.value;
							res
						})
						.cloned()
						.collect(),
				);
			}
		} else {
			None
		}
	}
}
