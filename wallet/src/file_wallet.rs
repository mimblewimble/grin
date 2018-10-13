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
use std::marker;
use std::path::{Path, MAIN_SEPARATOR};

use serde_json;
use tokio_core::reactor;
use tokio_retry::strategy::FibonacciBackoff;
use tokio_retry::Retry;
use uuid::Uuid;

use failure::ResultExt;

use keychain::{self, Identifier, Keychain};
use util::secp::pedersen;
use util::LOGGER;

use error::{Error, ErrorKind};

use libwallet;

use libwallet::types::{
	Context, OutputData, TxLogEntry, WalletBackend, WalletClient, WalletDetails, WalletOutputBatch,
};

use types::{WalletConfig, WalletSeed};

const DETAIL_FILE: &'static str = "wallet.det";
const DET_BCK_FILE: &'static str = "wallet.detbck";
const DAT_FILE: &'static str = "wallet.dat";
const BCK_FILE: &'static str = "wallet.bck";
const LOCK_FILE: &'static str = "wallet.lock";

#[derive(Debug)]
struct FileBatch<'a, K: 'a>
where
	K: Keychain,
{
	/// List of outputs
	outputs: &'a mut HashMap<String, OutputData>,
	/// Wallet Details
	details: WalletDetails,
	/// Data file path
	data_file_path: String,
	/// Details file path
	details_file_path: String,
	/// lock file path
	lock_file_path: String,
	/// PhantomData for our K: Keychain.
	_marker: marker::PhantomData<K>,
}

impl<'a, K> WalletOutputBatch<K> for FileBatch<'a, K>
where
	K: Keychain,
{
	fn keychain(&mut self) -> &mut K {
		unimplemented!();
	}

	fn save(&mut self, out: OutputData) -> Result<(), libwallet::Error> {
		let _ = self.outputs.insert(out.key_id.to_hex(), out);
		Ok(())
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, libwallet::Error> {
		self.outputs
			.get(&id.to_hex())
			.map(|od| od.clone())
			.ok_or(libwallet::ErrorKind::Backend("not found".to_string()).into())
	}

	fn delete(&mut self, id: &Identifier) -> Result<(), libwallet::Error> {
		let _ = self.outputs.remove(&id.to_hex());
		Ok(())
	}

	fn save_details(&mut self, _r: Identifier, w: WalletDetails) -> Result<(), libwallet::Error> {
		self.details = w;
		Ok(())
	}

	fn next_tx_log_id(&mut self, _root_key_id: Identifier) -> Result<u32, libwallet::Error> {
		Ok(0)
	}

	fn save_tx_log_entry(&self, _t: TxLogEntry) -> Result<(), libwallet::Error> {
		// not implemented for file wallets
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

	fn iter(&self) -> Box<Iterator<Item = OutputData>> {
		unimplemented!()
	}

	fn tx_log_iter(&self) -> Box<Iterator<Item = TxLogEntry>> {
		unimplemented!()
	}

	fn save_private_context(
		&mut self,
		_slate_id: &[u8],
		_ctx: &Context,
	) -> Result<(), libwallet::Error> {
		unimplemented!()
	}

	fn delete_private_context(&mut self, _slate_id: &[u8]) -> Result<(), libwallet::Error> {
		unimplemented!()
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

impl<'a, K> Drop for FileBatch<'a, K>
where
	K: Keychain,
{
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
pub struct FileWallet<C, K> {
	/// Keychain
	pub keychain: Option<K>,
	/// Client implementation
	pub client: C,
	/// Configuration
	pub config: WalletConfig,
	/// passphrase: TODO better ways of dealing with this other than storing
	passphrase: String,
	/// List of outputs
	pub outputs: HashMap<String, OutputData>,
	/// Tx log
	pub tx_log: Vec<TxLogEntry>,
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

impl<C, K> WalletBackend<C, K> for FileWallet<C, K>
where
	C: WalletClient,
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
		Ok(())
	}

	/// Close wallet and remove any stored credentials (TBD)
	fn close(&mut self) -> Result<(), libwallet::Error> {
		debug!(LOGGER, "Closing wallet keychain");
		self.keychain = None;
		Ok(())
	}

	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K {
		self.keychain.as_mut().unwrap()
	}

	/// Return the client being used
	fn client(&mut self) -> &mut C {
		&mut self.client
	}

	fn iter<'a>(&'a self) -> Box<Iterator<Item = OutputData> + 'a> {
		Box::new(self.outputs.values().cloned())
	}

	fn get_tx_log_entry(&self, _u: &Uuid) -> Result<Option<TxLogEntry>, libwallet::Error> {
		Ok(None)
	}

	fn tx_log_iter<'a>(&'a self) -> Box<Iterator<Item = TxLogEntry> + 'a> {
		Box::new(self.tx_log.iter().cloned())
	}

	fn get_private_context(&mut self, _slate_id: &[u8]) -> Result<Context, libwallet::Error> {
		unimplemented!()
	}

	fn get(&self, id: &Identifier) -> Result<OutputData, libwallet::Error> {
		self.outputs
			.get(&id.to_hex())
			.map(|o| o.clone())
			.ok_or(libwallet::ErrorKind::Backend("not found".to_string()).into())
	}

	fn get_commitment(
		&mut self,
		_id: &Identifier,
	) -> Result<pedersen::Commitment, libwallet::Error> {
		unimplemented!();
	}

	fn batch<'a>(&'a mut self) -> Result<Box<WalletOutputBatch<K> + 'a>, libwallet::Error> {
		self.lock()?;

		// We successfully acquired the lock - so do what needs to be done.
		self.read_or_create_paths()
			.context(libwallet::ErrorKind::CallbackImpl("Lock Error"))?;
		self.write(&self.backup_file_path, &self.details_bak_path)
			.context(libwallet::ErrorKind::CallbackImpl("Write Error"))?;

		Ok(Box::new(FileBatch {
			outputs: &mut self.outputs,
			details: self.details.clone(),
			data_file_path: self.data_file_path.clone(),
			details_file_path: self.details_file_path.clone(),
			lock_file_path: self.lock_file_path.clone(),
			_marker: marker::PhantomData,
		}))
	}

	/// Next child index when we want to create a new output.
	fn next_child<'a>(
		&'a mut self,
		root_key_id: keychain::Identifier,
	) -> Result<u32, libwallet::Error> {
		let mut details = self.details(root_key_id.clone())?;
		let mut max_n = 0;
		for out in self.iter() {
			if max_n < out.n_child && out.root_key_id == root_key_id {
				max_n = out.n_child;
			}
		}
		let mut batch = self.batch()?;
		if details.last_child_index <= max_n {
			details.last_child_index = max_n + 1;
		} else {
			details.last_child_index += 1;
		}
		batch.save_details(root_key_id.clone(), details.clone())?;
		batch.commit()?;
		Ok(details.last_child_index)
	}

	fn details(&mut self, _root_key_id: Identifier) -> Result<WalletDetails, libwallet::Error> {
		self.batch()?;
		Ok(self.details.clone())
	}

	/// Restore wallet contents
	fn restore(&mut self) -> Result<(), libwallet::Error> {
		libwallet::internal::restore::restore(self)
	}
}

impl<C, K> FileWallet<C, K>
where
	C: WalletClient,
	K: Keychain,
{
	/// Create a new FileWallet instance
	pub fn new(config: WalletConfig, passphrase: &str, client: C) -> Result<Self, Error> {
		let mut retval = FileWallet {
			keychain: None,
			client: client,
			config: config.clone(),
			passphrase: String::from(passphrase),
			outputs: HashMap::new(),
			tx_log: Vec::new(),
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
		let retry_result = core
			.run(retry_future)
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
}
