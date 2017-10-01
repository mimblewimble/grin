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
use secp;
use secp::key::SecretKey;

use core::core::{Block, Transaction, TxKernel, Output, build};
use core::ser;
use api::{self, ApiEndpoint, Operation, ApiResult};
use extkey::ExtendedKey;
use types::*;
use util;

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
struct TxWrapper {
	tx_hex: String,
}

/// Receive an already well formed JSON transaction issuance and finalize the
/// transaction, adding our receiving output, to broadcast to the rest of the
/// network.
pub fn receive_json_tx(
	config: &WalletConfig,
	ext_key: &ExtendedKey,
	partial_tx_str: &str,
) -> Result<(), Error> {
	let (amount, blinding, partial_tx) = partial_tx_from_json(partial_tx_str)?;
	let final_tx = receive_transaction(&config, ext_key, amount, blinding, partial_tx)?;
	let tx_hex = util::to_hex(ser::ser_vec(&final_tx).unwrap());

	let url = format!("{}/v1/pool/push", config.check_node_api_http_addr.as_str());
	let _: () = api::client::post(url.as_str(), &TxWrapper { tx_hex: tx_hex })
		.map_err(|e| Error::Node(e))?;
	Ok(())
}

/// Component used to receive coins, implements all the receiving end of the
/// wallet REST API as well as some of the command-line operations.
#[derive(Clone)]
pub struct WalletReceiver {
	pub key: ExtendedKey,
	pub config: WalletConfig,
}

impl ApiEndpoint for WalletReceiver {
	type ID = String;
	type T = String;
	type OP_IN = WalletReceiveRequest;
	type OP_OUT = CbData;

	fn operations(&self) -> Vec<Operation> {
		vec![
			Operation::Custom("coinbase".to_string()),
			Operation::Custom("receive_json_tx".to_string()),
		]
	}

	fn operation(&self, op: String, input: WalletReceiveRequest) -> ApiResult<CbData> {
		match op.as_str() {
			"coinbase" => {
				match input {
					WalletReceiveRequest::Coinbase(cb_amount) => {
						debug!("Operation {} with amount {}", op, cb_amount.amount);
						if cb_amount.amount == 0 {
							return Err(api::Error::Argument(format!("Zero amount not allowed.")));
						}
						let (out, kern) = receive_coinbase(
							&self.config,
							&self.key,
							cb_amount.amount,
						).map_err(|e| {
							api::Error::Internal(format!("Error building coinbase: {:?}", e))
						})?;
						let out_bin = ser::ser_vec(&out).map_err(|e| {
							api::Error::Internal(format!("Error serializing output: {:?}", e))
						})?;
						let kern_bin = ser::ser_vec(&kern).map_err(|e| {
							api::Error::Internal(format!("Error serializing kernel: {:?}", e))
						})?;
						Ok(CbData {
							output: util::to_hex(out_bin),
							kernel: util::to_hex(kern_bin),
						})
					}
					_ => Err(api::Error::Argument(
						format!("Incorrect request data: {}", op),
					)),
				}
			}
			"receive_json_tx" => {
				match input {
					WalletReceiveRequest::PartialTransaction(partial_tx_str) => {
						debug!("Operation {} with transaction {}", op, &partial_tx_str);
						receive_json_tx(&self.config, &self.key, &partial_tx_str)
							.map_err(|e| {
								api::Error::Internal(
									format!("Error processing partial transaction: {:?}", e),
								)
							})
							.unwrap();

						// TODO: Return emptiness for now, should be a proper enum return type
						Ok(CbData {
							output: String::from(""),
							kernel: String::from(""),
						})
					}
					_ => Err(api::Error::Argument(
						format!("Incorrect request data: {}", op),
					)),
				}
			}
			_ => Err(api::Error::Argument(format!("Unknown operation: {}", op))),
		}
	}
}

/// Build a coinbase output and the corresponding kernel
fn receive_coinbase(
	config: &WalletConfig,
	ext_key: &ExtendedKey,
	amount: u64,
) -> Result<(Output, TxKernel), Error> {
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		// derive a new private for the reward
		let next_child = wallet_data.next_child(&ext_key.fingerprint);
		let coinbase_key = ext_key.derive(&secp, next_child).map_err(|e| Error::Key(e))?;

		// track the new output and return the stuff needed for reward
		wallet_data.append_output(OutputData {
			fingerprint: coinbase_key.fingerprint,
			n_child: coinbase_key.n_child,
			value: amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});
		debug!(
			"Using child {} for a new coinbase output.",
			coinbase_key.n_child
		);

		Block::reward_output(coinbase_key.key, &secp).map_err(&From::from)
	})?
}

/// Builds a full transaction from the partial one sent to us for transfer
fn receive_transaction(
	config: &WalletConfig,
	ext_key: &ExtendedKey,
	amount: u64,
	blinding: SecretKey,
	partial: Transaction,
) -> Result<Transaction, Error> {

	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		let next_child = wallet_data.next_child(&ext_key.fingerprint);
		let out_key = ext_key.derive(&secp, next_child).map_err(|e| Error::Key(e))?;

		// TODO - replace with real fee calculation
		// TODO - note we are not enforcing this in consensus anywhere yet
		let fee_amount = 1;
		let out_amount = amount - fee_amount;

		let (tx_final, _) = build::transaction(vec![
			build::initial_tx(partial),
			build::with_excess(blinding),
			build::output(out_amount, out_key.key),
			build::with_fee(fee_amount),
		])?;

		// make sure the resulting transaction is valid (could have been lied to
		// on excess)
		tx_final.validate(&secp)?;

		// track the new output and return the finalized transaction to broadcast
		wallet_data.append_output(OutputData {
			fingerprint: out_key.fingerprint,
			n_child: out_key.n_child,
			value: out_amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});

		debug!(
			"Using child {} for a new transaction output.",
			out_key.n_child
		);

		Ok(tx_final)
	})?
}
