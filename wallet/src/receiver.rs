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

//! Provides the JSON/HTTP API for wallets to receive payments. Because
//! receiving money in MimbleWimble requires an interactive exchange, a
//! wallet server that's running at all time is required in many cases.
//!
//! The API looks like this:
//!
//! POST /v1/wallet/receive
//! > {
//! >   "amount": 10,
//! >   "blind_sum": "a12b7f...",
//! >   "tx": "f083de...",
//! > }
//!
//! < {
//! <   "tx": "f083de...",
//! <   "status": "ok"
//! < }
//!
//! POST /v1/wallet/finalize
//! > {
//! >   "tx": "f083de...",
//! > }
//!
//! POST /v1/wallet/receive_coinbase
//! > {
//! >   "amount": 1,
//! > }
//!
//! < {
//! <   "output": "8a90bc...",
//! <   "kernel": "f083de...",
//! < }
//!
//! Note that while at this point the finalize call is completely unecessary, a
//! double-exchange will be required as soon as we support Schnorr signatures.
//! So we may as well have it in place already.

use std::convert::From;
use secp::{self, Secp256k1};
use secp::key::SecretKey;

use core::core::{Block, Transaction, TxKernel, Output, build};
use core::ser;
use api::{self, ApiEndpoint, Operation, ApiResult};
use extkey::{self, ExtendedKey};
use types::*;
use util;

/// Amount in request to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CbAmount {
	amount: u64,
}

/// Response to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CbData {
	output: String,
	kernel: String,
}

/// Component used to receive coins, implements all the receiving end of the
/// wallet REST API as well as some of the command-line operations.
#[derive(Clone)]
pub struct WalletReceiver {
	pub key: ExtendedKey,
}

impl ApiEndpoint for WalletReceiver {
	type ID = String;
	type T = String;
	type OP_IN = CbAmount;
	type OP_OUT = CbData;

	fn operations(&self) -> Vec<Operation> {
		vec![Operation::Custom("receive_coinbase".to_string())]
	}

	fn operation(&self, op: String, input: CbAmount) -> ApiResult<CbData> {
		debug!("Operation {} with amount {}", op, input.amount);
		if input.amount == 0 {
			return Err(api::Error::Argument(format!("Zero amount not allowed.")));
		}
		match op.as_str() {
			"receive_coinbase" => {
				let (out, kern) =
					receive_coinbase(&self.key, input.amount).map_err(|e| {
							api::Error::Internal(format!("Error building coinbase: {:?}", e))
						})?;
				let out_bin =
					ser::ser_vec(&out).map_err(|e| {
							api::Error::Internal(format!("Error serializing output: {:?}", e))
						})?;
				let kern_bin =
					ser::ser_vec(&kern).map_err(|e| {
							api::Error::Internal(format!("Error serializing kernel: {:?}", e))
						})?;
				Ok(CbData {
					output: util::to_hex(out_bin),
					kernel: util::to_hex(kern_bin),
				})
			}
			_ => Err(api::Error::Argument(format!("Unknown operation: {}", op))),
		}
	}
}

/// Build a coinbase output and the corresponding kernel
fn receive_coinbase(ext_key: &ExtendedKey, amount: u64) -> Result<(Output, TxKernel), Error> {
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	// derive a new private for the reward
	let mut wallet_data = WalletData::read_or_create()?;
	let next_child = wallet_data.next_child(ext_key.fingerprint);
	let coinbase_key = ext_key.derive(&secp, next_child).map_err(|e| Error::Key(e))?;

	// track the new output and return the stuff needed for reward
	wallet_data.append_output(OutputData {
		fingerprint: coinbase_key.fingerprint,
		n_child: coinbase_key.n_child,
		value: amount,
		status: OutputStatus::Unconfirmed,
	});
	wallet_data.write()?;

	info!("Using child {} for a new coinbase output.",
	      coinbase_key.n_child);

	Block::reward_output(ext_key.key, &secp).map_err(&From::from)
}
