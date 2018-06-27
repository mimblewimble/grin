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
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, MAIN_SEPARATOR};

use serde_json;
use tokio_core::reactor;
use tokio_retry::strategy::FibonacciBackoff;
use tokio_retry::Retry;

use failure::ResultExt;

use keychain::{self, Identifier, Keychain};
use util::secp::pedersen;
use util::LOGGER;

use error::{Error, ErrorKind};

use client;
use libtx::slate::Slate;
use libwallet;
use libwallet::types::{
	BlockFees, BlockIdentifier, CbData, MerkleProofWrapper, OutputData, TxWrapper, WalletBackend,
	WalletClient, WalletDetails, WalletOutputBatch,
};
use types::{WalletConfig, WalletSeed};

const DETAIL_FILE: &'static str = "wallet.det";
const DET_BCK_FILE: &'static str = "wallet.detbck";
const DAT_FILE: &'static str = "wallet.dat";
const BCK_FILE: &'static str = "wallet.bck";
const LOCK_FILE: &'static str = "wallet.lock";

#[derive(Debug)]
struct FileBatch<'a> {
	/// List of outputs
	outputs: &'a mut HashMap<String, OutputData>,
	/// Wallet Details
	details: &'a mut WalletDetails,
	/// Data file path
	data_file_path: String,
	/// Details file path
	details_file_path: String,
	/// lock file path
	lock_file_path: String,
}

impl<'a> WalletOutputBatch for FileBatch<'a> {
	fn save(&mut self, out: OutputData) -> Result<(), libwallet::Error> {
		let _ = self.outputs.insert(out.key_id.to_hex(), out);
		Ok(())
	}

	fn details(&mut self) -> &mut WalletDetails {
		&mut self.details
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, libwallet::Error> {
		self.outputs
			.get(&id.to_hex())
			.map(|od| od.clone())
			.ok_or(libwallet::ErrorKind::Backend("not found".to_string()).into())
	}

	fn iter<'b>(&'b self) -> Box<Iterator<Item = OutputData> + 'b> {
		Box::new(self.outputs.values().cloned())
	}

	fn delete(&mut self, id: &Identifier) -> Result<(), libwallet::Error> {
		let _ = self.outputs.remove(&id.to_hex());
		Ok(())
	}

	fn lock_output(&mut self, out: &mut OutputData) -> Result<(), libwallet::Error> {
		if let Some(out_to_lock) = self.outputs.get_mut(&out.key_id.to_hex()) {
			if out_to_lock.value == out.value {
				out_to_lock.lock()
			}
		}
		Ok(())
	}

	fn commit(&self) -> Result<(), libwallet::Error> {
		let mut data_file = File::create(self.data_file_path.clone())
			.context(libwallet::ErrorKind::CallbackImpl("Could not create"))?;
		let mut details_file = File::create(self.details_file_path.clone())
			.context(libwallet::ErrorKind::CallbackImpl("Could not create"))?;
		let mut outputs = self.outputs.values().collect::<Vec<_>>();
		outputs.sort();
		let res_json = serde_json::to_vec_pretty(&outputs).context(
			libwallet::ErrorKind::CallbackImpl("Error serializing wallet output data"),
		)?;
		let details_res_json = serde_json::to_vec_pretty(&self.details).context(
			libwallet::ErrorKind::CallbackImpl("Error serializing wallet details data"),
		)?;
		data_file
			.write_all(res_json.as_slice())
			.context(libwallet::ErrorKind::CallbackImpl(
				"Error writing wallet data file",
			))?;
		details_file
			.write_all(details_res_json.as_slice())
			.context(libwallet::ErrorKind::CallbackImpl(
				"Error writing wallet details file",
			))?;
		Ok(())
	}
}

impl<'a> Drop for FileBatch<'a> {
	fn drop(&mut self) {
		// delete the lock file
		if let Err(e) = fs::remove_dir(&self.lock_file_path) {
			error!(
				LOGGER,
				"Could not remove wallet lock file. Maybe insufficient rights? {:?} ", e
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
	/// Details
	pub details: WalletDetails,
	/// Data file path
	pub data_file_path: String,
	/// Backup file path
	pub backup_file_path: String,
	/// lock file path
	pub lock_file_path: String,
	/// details file path
	pub details_file_path: String,
	/// Details backup file path
	pub details_bak_path: String,
}

impl<K> WalletBackend<K> for FileWallet<K>
where
	K: Keychain,
{
	/// Initialize with whatever stored credentials we have
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

	fn iter<'a>(&'a self) -> Box<Iterator<Item = OutputData> + 'a> {
		Box::new(self.outputs.values().cloned())
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, libwallet::Error> {
		self.outputs
			.get(&id.to_hex())
			.map(|o| o.clone())
			.ok_or(libwallet::ErrorKind::Backend("not found".to_string()).into())
	}

	fn batch<'a>(&'a mut self) -> Result<Box<WalletOutputBatch + 'a>, libwallet::Error> {
		self.lock()?;

		// We successfully acquired the lock - so do what needs to be done.
		self.read_or_create_paths()
			.context(libwallet::ErrorKind::CallbackImpl("Lock Error"))?;
		self.write(&self.backup_file_path, &self.details_bak_path)
			.context(libwallet::ErrorKind::CallbackImpl("Write Error"))?;

		Ok(Box::new(FileBatch {
			outputs: &mut self.outputs,
			details: &mut self.details,
			data_file_path: self.data_file_path.clone(),
			details_file_path: self.details_file_path.clone(),
			lock_file_path: self.lock_file_path.clone(),
		}))
	}

	/// Next child index when we want to create a new output.
	fn next_child<'a>(
		&'a mut self,
		root_key_id: keychain::Identifier,
	) -> Result<u32, libwallet::Error> {
		let mut batch = self.batch()?;
		{
			let mut max_n = 0;
			for out in batch.iter() {
				if max_n < out.n_child && out.root_key_id == root_key_id {
					max_n = out.n_child;
				}
			}
			let details = batch.details();
			if details.last_child_index <= max_n {
				details.last_child_index = max_n + 1;
			} else {
				details.last_child_index += 1;
			}
		}
		batch.commit()?;
		Ok(batch.details().last_child_index)
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
		// greedy. But if max_outputs(500) is actually not enough to cover the whole
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

	/// Return current metadata
	fn details(&mut self) -> &mut WalletDetails {
		&mut self.details
	}

	/// Restore wallet contents
	fn restore(&mut self) -> Result<(), libwallet::Error> {
		libwallet::internal::restore::restore(self)
	}
}

impl<K> WalletClient for FileWallet<K> {
	/// Return URL for check node
	fn node_url(&self) -> &str {
		&self.config.check_node_api_http_addr
	}

	/// Call the wallet API to create a coinbase transaction
	fn create_coinbase(&self, block_fees: &BlockFees) -> Result<CbData, libwallet::Error> {
		let res = client::create_coinbase(self.node_url(), block_fees);
		match res {
			Ok(r) => Ok(r),
			Err(e) => {
				let message = format!("{}", e.cause().unwrap());
				error!(
					LOGGER,
					"Create Coinbase: Communication error: {},{}",
					e.cause().unwrap(),
					e.backtrace().unwrap()
				);
				Err(libwallet::ErrorKind::WalletComms(message))?
			}
		}
	}

	/// Send a transaction slate to another listening wallet and return result
	fn send_tx_slate(&self, addr: &str, slate: &Slate) -> Result<Slate, libwallet::Error> {
		let res = client::send_tx_slate(addr, slate);
		match res {
			Ok(r) => Ok(r),
			Err(e) => {
				let message = format!("{}", e.cause().unwrap());
				error!(
					LOGGER,
					"Send TX Slate: Communication error: {},{}",
					e.cause().unwrap(),
					e.backtrace().unwrap()
				);
				Err(libwallet::ErrorKind::WalletComms(message))?
			}
		}
	}

	/// Posts a transaction to a grin node
	fn post_tx(&self, tx: &TxWrapper, fluff: bool) -> Result<(), libwallet::Error> {
		let res = client::post_tx(self.node_url(), tx, fluff).context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieves the current tip from the specified grin node
	fn get_chain_height(&self) -> Result<u64, libwallet::Error> {
		let res = client::get_chain_height(self.node_url()).context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieve a list of outputs from the specified grin node
	/// need "by_height" and "by_id" variants
	fn get_outputs_from_node(
		&self,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, String>, libwallet::Error> {
		let res = client::get_outputs_from_node(self.node_url(), wallet_outputs)
			.context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// Outputs by PMMR index
	fn get_outputs_by_pmmr_index(
		&self,
		start_height: u64,
		max_outputs: u64,
	) -> Result<
		(
			u64,
			u64,
			Vec<(pedersen::Commitment, pedersen::RangeProof, bool)>,
		),
		libwallet::Error,
	> {
		let res = client::get_outputs_by_pmmr_index(self.node_url(), start_height, max_outputs)
			.context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// Get any missing block hashes from node
	fn get_missing_block_hashes_from_node(
		&self,
		height: u64,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<
		(
			HashMap<pedersen::Commitment, (u64, BlockIdentifier)>,
			HashMap<pedersen::Commitment, MerkleProofWrapper>,
		),
		libwallet::Error,
	> {
		let res =
			client::get_missing_block_hashes_from_node(self.node_url(), height, wallet_outputs)
				.context(libwallet::ErrorKind::Node)?;
		Ok(res)
	}

	/// retrieve merkle proof for a commit from a node
	fn create_merkle_proof(&self, commit: &str) -> Result<MerkleProofWrapper, libwallet::Error> {
		let res = client::create_merkle_proof(self.node_url(), commit)
			.context(libwallet::ErrorKind::Node)?;
		Ok(res)
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
			details: WalletDetails::default(),
			data_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, DAT_FILE),
			backup_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, BCK_FILE),
			lock_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, LOCK_FILE),
			details_file_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, DETAIL_FILE),
			details_bak_path: format!("{}{}{}", config.data_file_dir, MAIN_SEPARATOR, DET_BCK_FILE),
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
		if Path::new(&self.details_file_path.clone()).exists() {
			self.details = self.read_details()?;
		}
		Ok(())
	}

	/// Read details file from disk
	fn read_details(&self) -> Result<WalletDetails, Error> {
		let details_file = File::open(self.details_file_path.clone())
			.context(ErrorKind::FileWallet(&"Could not open wallet details file"))?;
		serde_json::from_reader(details_file)
			.context(ErrorKind::Format)
			.map_err(|e| e.into())
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

	/// Write the wallet and details data to disk.
	fn write(&self, data_file_path: &str, details_file_path: &str) -> Result<(), Error> {
		let mut data_file =
			File::create(data_file_path).context(ErrorKind::FileWallet(&"Could not create "))?;
		let mut outputs = self.outputs.values().collect::<Vec<_>>();
		outputs.sort();
		let res_json = serde_json::to_vec_pretty(&outputs)
			.context(ErrorKind::FileWallet("Error serializing wallet data"))?;
		data_file
			.write_all(res_json.as_slice())
			.context(ErrorKind::FileWallet(&"Error writing wallet file"))?;
		// write details file
		let mut details_file =
			File::create(details_file_path).context(ErrorKind::FileWallet(&"Could not create "))?;
		let res_json = serde_json::to_string_pretty(&self.details).context(ErrorKind::FileWallet(
			"Error serializing wallet details file",
		))?;
		details_file
			.write_all(res_json.into_bytes().as_slice())
			.context(ErrorKind::FileWallet(&"Error writing wallet details file"))
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
