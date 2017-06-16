// Copyright 2016 The Grin Developers
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

use std::convert::From;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::num;
use std::path::Path;
use std::path::MAIN_SEPARATOR;

use serde_json;

use secp;
use secp::key::SecretKey;

use api;
use core::core::Transaction;
use core::ser;
use extkey;
use util;

const DAT_FILE: &'static str = "wallet.dat";
const LOCK_FILE: &'static str = "wallet.lock";

/// Wallet errors, mostly wrappers around underlying crypto or I/O errors.
#[derive(Debug)]
pub enum Error {
	NotEnoughFunds(u64),
	Crypto(secp::Error),
	Key(extkey::Error),
	WalletData(String),
	/// An error in the format of the JSON structures exchanged by the wallet
	Format(String),
	/// Error when contacting a node through its API
	Node(api::Error),
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Crypto(e)
	}
}

impl From<extkey::Error> for Error {
	fn from(e: extkey::Error) -> Error {
		Error::Key(e)
	}
}

impl From<serde_json::Error> for Error {
	fn from(e: serde_json::Error) -> Error {
		Error::Format(e.to_string())
	}
}

impl From<num::ParseIntError> for Error {
	fn from(e: num::ParseIntError) -> Error {
		Error::Format("Invalid hex".to_string())
	}
}

impl From<api::Error> for Error {
	fn from(e: api::Error) -> Error {
		Error::Node(e)
	}
}

#[derive(Debug, Clone)]
pub struct WalletConfig {
	pub api_http_addr: String,
	pub data_file_dir: String,
}

impl Default for WalletConfig {
	fn default() -> WalletConfig {
		WalletConfig { 
			api_http_addr: "http://127.0.0.1:13415".to_string(),
			data_file_dir: ".".to_string(),
		}
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

/// Information about an output that's being tracked by the wallet. Must be
/// enough to reconstruct the commitment associated with the ouput when the
/// root private key is known.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OutputData {
	/// Private key fingerprint (in case the wallet tracks multiple)
	pub fingerprint: [u8; 4],
	/// How many derivations down from the root key
	pub n_child: u32,
	/// Value of the output, necessary to rebuild the commitment
	pub value: u64,
	/// Current status of the output
	pub status: OutputStatus,
}

impl OutputData {
	/// Lock a given output to avoid conflicting use
	pub fn lock(&mut self) {
		self.status = OutputStatus::Locked;
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
	pub outputs: Vec<OutputData>,
}

impl WalletData {
	/// Allows the reading and writing of the wallet data within a file lock.
	/// Just provide a closure taking a mutable WalletData. The lock should
  /// be held for as short a period as possible to avoid contention.
	/// Note that due to the impossibility to do an actual file lock easily
	/// across operating systems, this just creates a lock file with a "should
  /// not exist" option.
	pub fn with_wallet<T, F>(data_file_dir:&str, f: F) -> Result<T, Error>
		where F: FnOnce(&mut WalletData) -> T
	{
		//create directory if it doesn't exist
		fs::create_dir_all(data_file_dir).unwrap_or_else(|why| {
        	info!("! {:?}", why.kind());
    	});
		
		let data_file_path = &format!("{}{}{}", data_file_dir, MAIN_SEPARATOR, DAT_FILE);
		let lock_file_path = &format!("{}{}{}", data_file_dir, MAIN_SEPARATOR, LOCK_FILE);

		// create the lock files, if it already exists, will produce an error
    	OpenOptions::new().write(true).create_new(true).open(lock_file_path).map_err(|e| {
				Error::WalletData(format!("Could not create wallet lock file. Either \
            some other process is using the wallet or there's a write access \
            issue."))
    	})?;

		// do what needs to be done
		let mut wdat = WalletData::read_or_create(data_file_path)?;
		let res = f(&mut wdat);
		wdat.write(data_file_path)?;

		// delete the lock file
		fs::remove_file(lock_file_path).map_err(|e| {
				Error::WalletData(format!("Could not remove wallet lock file. Maybe insufficient \
				                           rights?"))
			})?;

		Ok(res)
	}

	/// Read the wallet data or created a brand new one if it doesn't exist yet
	fn read_or_create(data_file_path:&str) -> Result<WalletData, Error> {
		if Path::new(data_file_path).exists() {
			WalletData::read(data_file_path)
		} else {
			// just create a new instance, it will get written afterward
			Ok(WalletData { outputs: vec![] })
		}
	}

	/// Read the wallet data from disk.
	fn read(data_file_path:&str) -> Result<WalletData, Error> {
		let data_file = File::open(data_file_path)
      .map_err(|e| Error::WalletData(format!("Could not open {}: {}", data_file_path, e)))?;
		serde_json::from_reader(data_file)
			.map_err(|e| Error::WalletData(format!("Error reading {}: {}", data_file_path, e)))
	}

	/// Write the wallet data to disk.
	fn write(&self, data_file_path:&str) -> Result<(), Error> {
		let mut data_file = File::create(data_file_path)
      .map_err(|e| Error::WalletData(format!("Could not create {}: {}", data_file_path, e)))?;
		let res_json = serde_json::to_vec_pretty(self)
      .map_err(|_| Error::WalletData(format!("Error serializing wallet data.")))?;
		data_file.write_all(res_json.as_slice())
			.map_err(|e| Error::WalletData(format!("Error writing {}: {}", data_file_path, e)))
	}

	/// Append a new output information to the wallet data.
	pub fn append_output(&mut self, out: OutputData) {
		self.outputs.push(out);
	}

	/// Select a subset of unspent outputs to spend in a transaction
	/// transferring
	/// the provided amount.
	pub fn select(&self, fingerprint: [u8; 4], amount: u64) -> (Vec<OutputData>, i64) {
		let mut to_spend = vec![];
		let mut input_total = 0;
		// TODO very naive impl for now, there's definitely better coin selection
		// algos available
		for out in &self.outputs {
			if out.status == OutputStatus::Unspent && out.fingerprint == fingerprint {
				to_spend.push(out.clone());
				input_total += out.value;
				if input_total >= amount {
					break;
				}
			}
		}
		(to_spend, (input_total as i64) - (amount as i64))
	}

	/// Next child index when we want to create a new output.
	pub fn next_child(&self, fingerprint: [u8; 4]) -> u32 {
		let mut max_n = 0;
		for out in &self.outputs {
			if max_n < out.n_child && out.fingerprint == fingerprint {
				max_n = out.n_child;
			}
		}
		max_n + 1
	}
}

/// Helper in serializing the information a receiver requires to build a
/// transaction.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct JSONPartialTx {
	amount: u64,
	blind_sum: String,
	tx: String,
}

/// Encodes the information for a partial transaction (not yet completed by the
/// receiver) into JSON.
pub fn partial_tx_to_json(receive_amount: u64, blind_sum: SecretKey, tx: Transaction) -> String {
	let partial_tx = JSONPartialTx {
		amount: receive_amount,
		blind_sum: util::to_hex(blind_sum.as_ref().to_vec()),
		tx: util::to_hex(ser::ser_vec(&tx).unwrap()),
	};
	serde_json::to_string_pretty(&partial_tx).unwrap()
}

/// Reads a partial transaction encoded as JSON into the amount, sum of blinding
/// factors and the transaction itself.
pub fn partial_tx_from_json(json_str: &str) -> Result<(u64, SecretKey, Transaction), Error> {
	let partial_tx: JSONPartialTx = serde_json::from_str(json_str)?;

	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let blind_bin = util::from_hex(partial_tx.blind_sum)?;
	let blinding = SecretKey::from_slice(&secp, &blind_bin[..])?;
	let tx_bin = util::from_hex(partial_tx.tx)?;
	let tx =
		ser::deserialize(&mut &tx_bin[..]).map_err(|_| {
				Error::Format("Could not deserialize transaction, invalid format.".to_string())
			})?;

	Ok((partial_tx.amount, blinding, tx))
}
