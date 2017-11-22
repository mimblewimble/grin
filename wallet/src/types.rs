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

use blake2;
use rand::{thread_rng, Rng};
use std::{error, fmt, num};
use std::convert::From;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::path::MAIN_SEPARATOR;
use std::collections::HashMap;
use std::cmp::min;

use hyper;
use serde_json;
use util::secp;
use tokio_core::reactor;
use tokio_retry::Retry;
use tokio_retry::strategy::FibonacciBackoff;


use api;
use core::consensus;
use core::core::{transaction, Transaction};
use core::ser;
use keychain;
use util;
use util::LOGGER;

const DAT_FILE: &'static str = "wallet.dat";
const LOCK_FILE: &'static str = "wallet.lock";
const SEED_FILE: &'static str = "wallet.seed";

const DEFAULT_BASE_FEE: u64 = consensus::MILLI_GRIN;

/// Transaction fee calculation
pub fn tx_fee(input_len: usize, output_len: usize, base_fee: Option<u64>) -> u64 {
	let use_base_fee = match base_fee {
		Some(bf) => bf,
		None => DEFAULT_BASE_FEE,
	};
	let mut tx_weight = -1 * (input_len as i32) + 4 * (output_len as i32) + 1;
	if tx_weight < 1 {
		tx_weight = 1;
	}

	(tx_weight as u64) * use_base_fee
}

/// Wallet errors, mostly wrappers around underlying crypto or I/O errors.
#[derive(Debug)]
pub enum Error {
	NotEnoughFunds(u64),
	FeeDispute { sender_fee: u64, recipient_fee: u64 },
	Keychain(keychain::Error),
	Transaction(transaction::Error),
	Secp(secp::Error),
	WalletData(String),
	/// An error in the format of the JSON structures exchanged by the wallet
	Format(String),
	/// An IO Error
	IOError(io::Error),
	/// Error when contacting a node through its API
	Node(api::Error),
	/// Error originating from hyper.
	Hyper(hyper::Error),
	/// Error originating from hyper uri parsing.
	Uri(hyper::error::UriError),
}

impl error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			_ => "some kind of wallet error",
		}
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			_ => write!(f, "some kind of wallet error"),
		}
	}
}

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<transaction::Error> for Error {
	fn from(e: transaction::Error) -> Error {
		Error::Transaction(e)
	}
}

impl From<serde_json::Error> for Error {
	fn from(e: serde_json::Error) -> Error {
		Error::Format(e.to_string())
	}
}

impl From<num::ParseIntError> for Error {
	fn from(_: num::ParseIntError) -> Error {
		Error::Format("Invalid hex".to_string())
	}
}

impl From<api::Error> for Error {
	fn from(e: api::Error) -> Error {
		Error::Node(e)
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::IOError(e)
	}
}

impl From<hyper::Error> for Error {
	fn from(e: hyper::Error) -> Error {
		Error::Hyper(e)
	}
}

impl From<hyper::error::UriError> for Error {
	fn from(e: hyper::error::UriError) -> Error {
		Error::Uri(e)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
	// Whether to run a wallet
	pub enable_wallet: bool,
	// The api interface/ip_address that this api server (i.e. this wallet) will run
	// by default this is 127.0.0.1 (and will not accept connections from external clients)
	pub api_listen_interface: String,
	// The port this wallet will run on
	pub api_listen_port: String,
	// The api address of a running server node against which transaction inputs
	// will be checked during send
	pub check_node_api_http_addr: String,
	// The directory in which wallet files are stored
	pub data_file_dir: String,
}

impl Default for WalletConfig {
	fn default() -> WalletConfig {
		WalletConfig {
			enable_wallet: false,
			api_listen_interface: "127.0.0.1".to_string(),
			api_listen_port: "13415".to_string(),
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

/// Status of an output that's being tracked by the wallet. Can either be
/// unconfirmed, spent, unspent, or locked (when it's been used to generate
/// a transaction but we don't have confirmation that the transaction was
/// broadcasted or mined).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum OutputStatus {
	Unconfirmed,
	Unspent,
	Locked,
	Spent,
}

impl fmt::Display for OutputStatus {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			OutputStatus::Unconfirmed => write!(f, "Unconfirmed"),
			OutputStatus::Unspent => write!(f, "Unspent"),
			OutputStatus::Locked => write!(f, "Locked"),
			OutputStatus::Spent => write!(f, "Spent"),
		}
	}
}

/// Information about an output that's being tracked by the wallet. Must be
/// enough to reconstruct the commitment associated with the ouput when the
/// root private key is known.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OutputData {
	/// Root key_id that the key for this output is derived from
	pub root_key_id: keychain::Identifier,
	/// Derived key for this output
	pub key_id: keychain::Identifier,
	/// How many derivations down from the root key
	pub n_child: u32,
	/// Value of the output, necessary to rebuild the commitment
	pub value: u64,
	/// Current status of the output
	pub status: OutputStatus,
	/// Height of the output
	pub height: u64,
	/// Height we are locked until
	pub lock_height: u64,
	/// Is this a coinbase output? Is it subject to coinbase locktime?
	pub is_coinbase: bool,
}

impl OutputData {
	/// Lock a given output to avoid conflicting use
	fn lock(&mut self) {
		self.status = OutputStatus::Locked;
	}

	/// How many confirmations has this output received?
	/// If height == 0 then we are either Unconfirmed or the output was
	/// cut-through
	/// so we do not actually know how many confirmations this output had (and
	/// never will).
	pub fn num_confirmations(&self, current_height: u64) -> u64 {
		if self.status == OutputStatus::Unconfirmed {
			0
		} else if self.status == OutputStatus::Spent && self.height == 0 {
			0
		} else {
			// if an output has height n and we are at block n
			// then we have a single confirmation (the block it originated in)
			1 + (current_height - self.height)
		}
	}

	/// Check if output is eligible to spend based on state and height and confirmations
	pub fn eligible_to_spend(&self, current_height: u64, minimum_confirmations: u64) -> bool {
		if [OutputStatus::Spent, OutputStatus::Locked].contains(&self.status) {
			return false;
		} else if self.status == OutputStatus::Unconfirmed && self.is_coinbase {
			return false;
		} else if self.lock_height > current_height {
			return false;
		} else if self.status == OutputStatus::Unspent
			&& self.num_confirmations(current_height) >= minimum_confirmations
		{
			return true;
		} else if self.status == OutputStatus::Unconfirmed && minimum_confirmations == 0 {
			return true;
		} else {
			return false;
		}
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
		let bytes = util::from_hex(hex.to_string())?;
		Ok(WalletSeed::from_bytes(&bytes))
	}

	pub fn to_hex(&self) -> String {
		util::to_hex(self.0.to_vec())
	}

	pub fn derive_keychain(&self, password: &str) -> Result<keychain::Keychain, Error> {
		let seed = blake2::blake2b::blake2b(64, &password.as_bytes(), &self.0);
		let result = keychain::Keychain::from_seed(seed.as_bytes())?;
		Ok(result)
	}

	pub fn init_new() -> WalletSeed {
		let seed: [u8; 32] = thread_rng().gen();
		WalletSeed(seed)
	}

	pub fn init_file(wallet_config: &WalletConfig) -> Result<WalletSeed, Error> {
		// create directory if it doesn't exist
		fs::create_dir_all(&wallet_config.data_file_dir)?;

		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir,
			MAIN_SEPARATOR,
			SEED_FILE,
		);

		debug!(LOGGER, "Generating wallet seed file at: {}", seed_file_path,);

		if Path::new(seed_file_path).exists() {
			panic!("wallet seed file already exists");
		} else {
			let seed = WalletSeed::init_new();
			let mut file = File::create(seed_file_path)?;
			file.write_all(&seed.to_hex().as_bytes())?;
			Ok(seed)
		}
	}

	pub fn from_file(wallet_config: &WalletConfig) -> Result<WalletSeed, Error> {
		// create directory if it doesn't exist
		fs::create_dir_all(&wallet_config.data_file_dir)?;

		let seed_file_path = &format!(
			"{}{}{}",
			wallet_config.data_file_dir,
			MAIN_SEPARATOR,
			SEED_FILE,
		);

		debug!(LOGGER, "Using wallet seed file at: {}", seed_file_path,);

		if Path::new(seed_file_path).exists() {
			let mut file = File::open(seed_file_path)?;
			let mut buffer = String::new();
			file.read_to_string(&mut buffer)?;
			let wallet_seed = WalletSeed::from_hex(&buffer)?;
			Ok(wallet_seed)
		} else {
			error!(
				LOGGER,
				"Run: \"grin wallet init\" to initialize a new wallet.",
			);
			panic!("wallet seed file does not yet exist (grin wallet init)");
		}
	}
}

/// Wallet information tracking all our outputs. Based on HD derivation and
/// avoids storing any key data, only storing output amounts and child index.
/// This data structure is directly based on the JSON representation stored
/// on disk, so selection algorithms are fairly primitive and non optimized.
///
/// TODO optimization so everything isn't O(n) or even O(n^2)
/// TODO account for fees
/// TODO write locks so files don't get overwritten
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WalletData {
	pub outputs: HashMap<String, OutputData>,
}

impl WalletData {
	/// Allows for reading wallet data (without needing to acquire the write
	/// lock).
	pub fn read_wallet<T, F>(data_file_dir: &str, f: F) -> Result<T, Error>
	where
		F: FnOnce(&WalletData) -> T,
	{
		// open the wallet readonly and do what needs to be done with it
		let data_file_path = &format!("{}{}{}", data_file_dir, MAIN_SEPARATOR, DAT_FILE);
		let wdat = WalletData::read_or_create(data_file_path)?;
		let res = f(&wdat);
		Ok(res)
	}

	/// Allows the reading and writing of the wallet data within a file lock.
	/// Just provide a closure taking a mutable WalletData. The lock should
	/// be held for as short a period as possible to avoid contention.
	/// Note that due to the impossibility to do an actual file lock easily
	/// across operating systems, this just creates a lock file with a "should
	/// not exist" option.
	pub fn with_wallet<T, F>(data_file_dir: &str, f: F) -> Result<T, Error>
	where
		F: FnOnce(&mut WalletData) -> T,
	{
		// create directory if it doesn't exist
		fs::create_dir_all(data_file_dir).unwrap_or_else(|why| {
			info!(LOGGER, "! {:?}", why.kind());
		});

		let data_file_path = &format!("{}{}{}", data_file_dir, MAIN_SEPARATOR, DAT_FILE);
		let lock_file_path = &format!("{}{}{}", data_file_dir, MAIN_SEPARATOR, LOCK_FILE);

		info!(LOGGER, "Acquiring wallet lock ...");

		let action = || {
			debug!(LOGGER, "Attempting to acquire wallet lock");
			OpenOptions::new()
				.write(true)
				.create_new(true)
				.open(lock_file_path)
		};

		// use tokio_retry to cleanly define some retry logic
		let mut core = reactor::Core::new().unwrap();
		let retry_strategy = FibonacciBackoff::from_millis(10).take(10);
		let retry_future = Retry::spawn(core.handle(), retry_strategy, action);
		let retry_result = core.run(retry_future);

		match retry_result {
			Ok(_) => {}
			Err(_) => {
				error!(
					LOGGER,
					"Failed to acquire wallet lock file (multiple retries)",
				);
				return Err(Error::WalletData(format!("Failed to acquire lock file")));
			}
		}

		// We successfully acquired the lock - so do what needs to be done.
		let mut wdat = WalletData::read_or_create(data_file_path)?;
		let res = f(&mut wdat);
		wdat.write(data_file_path)?;

		// delete the lock file
		fs::remove_file(lock_file_path).map_err(|_| {
			Error::WalletData(format!(
				"Could not remove wallet lock file. Maybe insufficient rights?"
			))
		})?;

		info!(LOGGER, "... released wallet lock");

		Ok(res)
	}

	/// Read the wallet data or created a brand new one if it doesn't exist yet
	fn read_or_create(data_file_path: &str) -> Result<WalletData, Error> {
		if Path::new(data_file_path).exists() {
			WalletData::read(data_file_path)
		} else {
			// just create a new instance, it will get written afterward
			Ok(WalletData {
				outputs: HashMap::new(),
			})
		}
	}

	/// Read the wallet data from disk.
	fn read(data_file_path: &str) -> Result<WalletData, Error> {
		let data_file = File::open(data_file_path).map_err(|e| {
			Error::WalletData(format!("Could not open {}: {}", data_file_path, e))
		})?;
		serde_json::from_reader(data_file).map_err(|e| {
			Error::WalletData(format!("Error reading {}: {}", data_file_path, e))
		})
	}

	/// Write the wallet data to disk.
	fn write(&self, data_file_path: &str) -> Result<(), Error> {
		let mut data_file = File::create(data_file_path).map_err(|e| {
			Error::WalletData(format!("Could not create {}: {}", data_file_path, e))
		})?;
		let res_json = serde_json::to_vec_pretty(self).map_err(|e| {
			Error::WalletData(format!("Error serializing wallet data: {}", e))
		})?;
		data_file.write_all(res_json.as_slice()).map_err(|e| {
			Error::WalletData(format!("Error writing {}: {}", data_file_path, e))
		})
	}

	/// Append a new output data to the wallet data.
	/// TODO - we should check for overwriting here - only really valid for
	/// unconfirmed coinbase
	pub fn add_output(&mut self, out: OutputData) {
		self.outputs.insert(out.key_id.to_hex(), out.clone());
	}

	// TODO - careful with this, only for Unconfirmed (maybe Locked)?
	pub fn delete_output(&mut self, id: &keychain::Identifier) {
		self.outputs.remove(&id.to_hex());
	}

	/// Lock an output data.
	/// TODO - we should track identifier on these outputs (not just n_child)
	pub fn lock_output(&mut self, out: &OutputData) {
		if let Some(out_to_lock) = self.outputs.get_mut(&out.key_id.to_hex()) {
			if out_to_lock.value == out.value {
				out_to_lock.lock()
			}
		}
	}

	pub fn get_output(&self, key_id: &keychain::Identifier) -> Option<&OutputData> {
		self.outputs.get(&key_id.to_hex())
	}

	/// Select spendable coins from the wallet.
	/// Default strategy is to spend the maximum number of outputs (up to max_outputs).
	/// Alternative strategy is to spend smallest outputs first but only as many as necessary.
	/// When we introduce additional strategies we should pass something other than a bool in.
	pub fn select(
		&self,
		root_key_id: keychain::Identifier,
		amount: u64,
		current_height: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		default_strategy: bool,
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
		if eligible.len() > max_outputs {
			for window in eligible.windows(max_outputs) {
				let eligible = window.iter().cloned().collect::<Vec<_>>();
				if let Some(outputs) = self.select_from(amount, default_strategy, eligible) {
					return outputs;
				}
			}
		} else {
			if let Some(outputs) = self.select_from(amount, default_strategy, eligible.clone()) {
				return outputs;
			}
		}

		// we failed to find a suitable set of outputs to spend,
		// so return the largest amount we can so we can provide guidance on what is possible
		eligible.reverse();
		eligible.iter().take(max_outputs).cloned().collect()
	}

	// Select the full list of outputs if we are using the default strategy.
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
					outputs.iter()
						.take_while(|out| {
							let res = selected_amount < amount;
							selected_amount += out.value;
							res
						})
						.cloned()
						.collect()
				);
			}
		} else {
			None
		}
	}

	/// Next child index when we want to create a new output.
	pub fn next_child(&self, root_key_id: keychain::Identifier) -> u32 {
		let mut max_n = 0;
		for out in self.outputs.values() {
			if max_n < out.n_child && out.root_key_id == root_key_id {
				max_n = out.n_child;
			}
		}
		max_n + 1
	}
}

/// Helper in serializing the information a receiver requires to build a
/// transaction.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PartialTx {
	amount: u64,
	blind_sum: String,
	tx: String,
}

/// Builds a PartialTx from data sent by a sender (not yet completed by the receiver).
pub fn build_partial_tx(
	receive_amount: u64,
	blind_sum: keychain::BlindingFactor,
	tx: Transaction,
) -> PartialTx {
	PartialTx {
		amount: receive_amount,
		blind_sum: util::to_hex(blind_sum.secret_key().as_ref().to_vec()),
		tx: util::to_hex(ser::ser_vec(&tx).unwrap()),
	}
}

/// Reads a partial transaction into the amount, sum of blinding
/// factors and the transaction itself.
pub fn read_partial_tx(
	keychain: &keychain::Keychain,
	partial_tx: &PartialTx,
) -> Result<(u64, keychain::BlindingFactor, Transaction), Error> {
	let blind_bin = util::from_hex(partial_tx.blind_sum.clone())?;
	let blinding = keychain::BlindingFactor::from_slice(keychain.secp(), &blind_bin[..])?;
	let tx_bin = util::from_hex(partial_tx.tx.clone())?;
	let tx = ser::deserialize(&mut &tx_bin[..]).map_err(|_| {
		Error::Format("Could not deserialize transaction, invalid format.".to_string())
	})?;
	Ok((partial_tx.amount, blinding, tx))
}

/// Amount in request to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum WalletReceiveRequest {
	Coinbase(BlockFees),
	PartialTransaction(String),
	Finalize(String),
}

/// Fees in block to use for coinbase amount calculation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockFees {
	pub fees: u64,
	pub height: u64,
	pub key_id: Option<keychain::Identifier>,
}

impl BlockFees {
	pub fn key_id(&self) -> Option<keychain::Identifier> {
		self.key_id.clone()
	}
}

/// Response to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CbData {
	pub output: String,
	pub kernel: String,
	pub key_id: String,
}
