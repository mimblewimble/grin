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

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::convert::From;

use serde::{Serialize, Deserialize};
use serde_json;

use secp::{self, Secp256k1};
use secp::key::SecretKey;

use core::core::Transaction;
use core::ser;
use extkey;
use util;

const DAT_FILE: &'static str = "wallet.dat";

/// Wallet errors, mostly wrappers around underlying crypto or I/O errors.
#[derive(Debug)]
pub enum Error {
	NotEnoughFunds(u64),
	Crypto(secp::Error),
	Key(extkey::Error),
	WalletData(String),
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
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WalletData {
	outputs: Vec<OutputData>,
}

impl WalletData {

  /// Read the wallet data or created a brand new one if it doesn't exist yet
  pub fn read_or_create() -> Result<WalletData, Error> {
    if Path::new(DAT_FILE).exists() {
      WalletData::read()
    } else {
      // just create a new instance, it will get written afterward
      Ok(WalletData { outputs: vec![] })
    }
  }

  /// Read the wallet data from disk.
	pub fn read() -> Result<WalletData, Error> {
		let mut data_file = File::open(DAT_FILE)
      .map_err(|e| Error::WalletData(format!("Could not open {}: {}", DAT_FILE, e)))?;
		serde_json::from_reader(data_file)
			.map_err(|e| Error::WalletData(format!("Error reading {}: {}", DAT_FILE, e)))
	}

  /// Write the wallet data to disk.
	pub fn write(&self) -> Result<(), Error> {
		let mut data_file = File::create(DAT_FILE)
      .map_err(|e| Error::WalletData(format!("Could not create {}: {}", DAT_FILE, e)))?;
		let res_json = serde_json::to_vec_pretty(self)
      .map_err(|_| Error::WalletData(format!("Error serializing wallet data.")))?;
		data_file.write_all(res_json.as_slice())
			.map_err(|e| Error::WalletData(format!("Error writing {}: {}", DAT_FILE, e)))
	}

  /// Append a new output information to the wallet data.
	pub fn append_output(&mut self, out: OutputData) {
		self.outputs.push(out);
	}

  /// Select a subset of unspent outputs to spend in a transaction transferring
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
