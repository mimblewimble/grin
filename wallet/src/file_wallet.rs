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

use std::collections::HashMap;
use std::collections::hash_map::Values;
use std::fs::{self, File};
use std::io::Write;
use std::path::MAIN_SEPARATOR;
use std::path::Path;

use serde_json;
use tokio_core::reactor;
use tokio_retry::Retry;
use tokio_retry::strategy::FibonacciBackoff;

use failure::ResultExt;

use keychain::{self, Identifier, Keychain};
use util::LOGGER;
use util::secp::pedersen;

use error::{Error, ErrorKind};

use client;
use libtx::slate::Slate;
use libwallet;
use libwallet::types::*;
use types::{WalletConfig, WalletSeed};

pub const DAT_FILE: &'static str = "wallet.dat";
pub const BCK_FILE: &'static str = "wallet.bck";
pub const LOCK_FILE: &'static str = "wallet.lock";

struct FileBatch<'a> {
	/// List of outputs
	outputs: &'a mut HashMap<String, OutputData>,
	/// Data file path
	data_file_path: String,
	/// lock file path
	lock_file_path: String,
}

impl<'a> WalletOutputBatch for FileBatch<'a> {
	fn save(&mut self, out: OutputData) {
		let _ = self.outputs.insert(out.key_id.to_hex(), out);
	}

	fn get(&self, id: &Identifier) -> Option<OutputData> {
		self.outputs.get(&id.to_hex()).map(|od| od.clone())
	}

	fn delete(&mut self, id: &Identifier) {
		let _ = self.outputs.remove(&id.to_hex());
	}

	fn lock_output(&mut self, out: &OutputData) {
		if let Some(out_to_lock) = self.outputs.get_mut(&out.key_id.to_hex()) {
			if out_to_lock.value == out.value {
				out_to_lock.lock()
			}
		}
	}

	fn commit(&self) -> Result<(), libwallet::Error> {
		let mut data_file = File::create(self.data_file_path.clone())
			.context(libwallet::ErrorKind::CallbackImpl("Could not create"))?;
		let mut outputs = self.outputs.values().collect::<Vec<_>>();
		outputs.sort();
		let res_json = serde_json::to_vec_pretty(&outputs).context(
			libwallet::ErrorKind::CallbackImpl("Error serializing wallet data"),
		)?;
		data_file
			.write_all(res_json.as_slice())
			.context(libwallet::ErrorKind::CallbackImpl(
				"Error writing wallet file",
			))
			.map_err(|e| e.into())
	}
}

impl<'a> Drop for FileBatch<'a> {
	fn drop(&mut self) {
		// delete the lock file
		if let Err(e) = fs::remove_dir(&self.lock_file_path) {
			error!(
				LOGGER,
				"Could not remove wallet lock file. Maybe insufficient rights? "
			);
		}
		info!(LOGGER, "... released wallet lock");
	}
}

/// Wallet information tracking all our outputs. Based on HD derivation and
/// avoids storing any key data, only storing output amounts and child index.
#[derive(Debug, Clone)]
pub struct FileWallet<K> {
	/// Keychain
	pub keychain: Option<K>,
	/// Configuration
	pub config: WalletConfig,
	/// passphrase: TODO better ways of dealing with this other than storing
	passphrase: String,
	/// List of outputs
	pub outputs: HashMap<String, OutputData>,
	/// Data file path
	pub data_file_path: String,
	/// Backup file path
	pub backup_file_path: String,
	/// lock file path
	pub lock_file_path: String,
}

impl<K> WalletBackend<K> for FileWallet<K>
where
	K: Keychain,
{
	/// Initialise with whatever stored credentials we have
	fn open_with_credentials(&mut self) -> Result<(), libwallet::Error> {
		let wallet_seed = WalletSeed::from_file(&self.config)
			.context(libwallet::ErrorKind::CallbackImpl("Error opening wallet"))?;
		let keychain = wallet_seed.derive_keychain(&self.passphrase);
		self.keychain = Some(keychain.context(libwallet::ErrorKind::CallbackImpl(
			"Error deriving keychain",
		))?);
		// Just blow up password for now after it's been used
		self.passphrase = String::from("");
		Ok(())
	}

	/// Close wallet and remove any stored credentials (TBD)
	fn close(&mut self) -> Result<(), libwallet::Error> {
		self.keychain = None;
		Ok(())
	}

	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K {
		self.keychain.as_mut().unwrap()
	}

	fn iter<'a>(&'a self) -> Box<Iterator<Item = &'a OutputData> + 'a> {
		Box::new(self.outputs.values())
	}

	fn get(&self, id: &Identifier) -> Option<OutputData> {
		self.outputs.get(&id.to_hex()).map(|o| o.clone())
	}

	fn batch<'a>(&'a mut self) -> Result<Box<WalletOutputBatch + 'a>, libwallet::Error> {
		self.lock()?;

		// We successfully acquired the lock - so do what needs to be done.
		self.read_or_create_paths()
			.context(libwallet::ErrorKind::CallbackImpl("Lock Error"))?;
		self.write(&self.backup_file_path)
			.context(libwallet::ErrorKind::CallbackImpl("Write Error"))?;

		Ok(Box::new(FileBatch {
			outputs: &mut self.outputs,
			data_file_path: self.data_file_path.clone(),
			lock_file_path: self.lock_file_path.clone(),
		}))
	}

	/// Next child index when we want to create a new output.
	fn next_child(&self, root_key_id: keychain::Identifier) -> Result<u32, libwallet::Error> {
		let mut max_n = 0;
		for out in self.outputs.values() {
			if max_n < out.n_child && out.root_key_id == root_key_id {
				max_n = out.n_child;
			}
		}
		Ok(max_n + 1)
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

	/// Restore wallet contents
	fn restore(&mut self) -> Result<(), libwallet::Error> {
		libwallet::internal::restore::restore(self).context(libwallet::ErrorKind::Restore)?;
		Ok(())
	}
}

impl<K> WalletClient for FileWallet<K> {
	/// Return URL for check node
	fn node_url(&self) -> &str {
		&self.config.check_node_api_http_addr
	}

	/// Call the wallet API to create a coinbase transaction
	fn create_coinbase(
		&self,
		dest: &str,
		block_fees: &BlockFees,
	) -> Result<CbData, libwallet::Error> {
		let res =
			client::create_coinbase(dest, block_fees).context(libwallet::ErrorKind::WalletComms)?;
		Ok(res)
	}

	/// Send a transaction slate to another listening wallet and return result
	fn send_tx_slate(&self, dest: &str, slate: &Slate) -> Result<Slate, libwallet::Error> {
		let res = client::send_tx_slate(dest, slate).context(libwallet::ErrorKind::WalletComms)?;
		Ok(res)
	}

	/// Posts a tranaction to a grin node
	fn post_tx(&self, dest: &str, tx: &TxWrapper, fluff: bool) -> Result<(), libwallet::Error> {
		let res = client::post_tx(dest, tx, fluff).context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieves the current tip from the specified grin node
	fn get_chain_height(&self, addr: &str) -> Result<u64, libwallet::Error> {
		let res = client::get_chain_height(addr).context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieve a list of outputs from the specified grin node
	/// need "by_height" and "by_id" variants
	fn get_outputs_from_node(
		&self,
		addr: &str,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, String>, libwallet::Error> {
		let res = client::get_outputs_from_node(addr, wallet_outputs)
			.context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// Get any missing block hashes from node
	fn get_missing_block_hashes_from_node(
		&self,
		addr: &str,
		height: u64,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<
		(
			HashMap<pedersen::Commitment, (u64, BlockIdentifier)>,
			HashMap<pedersen::Commitment, MerkleProofWrapper>,
		),
		libwallet::Error,
	> {
		let res = client::get_missing_block_hashes_from_node(addr, height, wallet_outputs)
			.context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieve merkle proof for a commit from a node
	fn get_merkle_proof_for_commit(
		&self,
		addr: &str,
		commit: &str,
	) -> Result<MerkleProofWrapper, libwallet::Error> {
		Err(libwallet::ErrorKind::GenericError("Not Implemented"))?
	}
}

impl<K> FileWallet<K>
where
	K: Keychain,
{
	/// Create a new FileWallet instance
	pub fn new(config: WalletConfig, passphrase: &str) -> Result<Self, Error> {
		let mut retval = FileWallet {
			keychain: None,
			config: config.clone(),
			passphrase: String::from(passphrase),
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

	fn lock(&self) -> Result<(), libwallet::Error> {
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
		let retry_result = core.run(retry_future)
			.context(libwallet::ErrorKind::CallbackImpl(
				"Failed to acquire lock file",
			));

		match retry_result {
			Ok(_) => Ok(()),
			Err(e) => {
				error!(
					LOGGER,
					"Failed to acquire wallet lock file (multiple retries)",
				);
				Err(e.into())
			}
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
		serde_json::from_reader(data_file)
			.context(ErrorKind::Format)
			.map_err(|e| e.into())
	}

	/// Populate wallet_data with output_data from disk.
	fn read(&mut self) -> Result<(), Error> {
		let outputs = self.read_outputs()?;
		self.outputs = HashMap::new();
		for out in outputs {
			self.outputs.insert(out.key_id.to_hex(), out.clone());
		}
		Ok(())
	}

	/// Write the wallet data to disk.
	fn write(&self, data_file_path: &str) -> Result<(), Error> {
		let mut data_file =
			File::create(data_file_path).context(ErrorKind::FileWallet(&"Could not create "))?;
		let mut outputs = self.outputs.values().collect::<Vec<_>>();
		outputs.sort();
		let res_json = serde_json::to_vec_pretty(&outputs)
			.context(ErrorKind::FileWallet("Error serializing wallet data"))?;
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
