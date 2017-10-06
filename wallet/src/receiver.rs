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

use core::consensus::reward;
use core::core::{Block, Transaction, TxKernel, Output, build};
use core::ser;
use api::{self, ApiEndpoint, Operation, ApiResult};
use types::*;
use util;
use keychain::{BlindingFactor, Keychain};

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
	keychain: &Keychain,
	partial_tx_str: &str,
) -> Result<(), Error> {
	let (amount, blinding, partial_tx) = partial_tx_from_json(keychain, partial_tx_str)?;
	let final_tx = receive_transaction(config, keychain, amount, blinding, partial_tx)?;
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
	pub keychain: Keychain,
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
					WalletReceiveRequest::Coinbase(cb_fees) => {
						debug!("Operation {} with fees {:?}", op, cb_fees);
						let (out, kern, derivation) =
							receive_coinbase(
								&self.config,
								&self.keychain,
								cb_fees.fees,
								cb_fees.derivation,
							).map_err(|e| {
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
							derivation: derivation,
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
						receive_json_tx(&self.config, &self.keychain, &partial_tx_str).map_err(|e| {
							api::Error::Internal(format!("Error processing partial transaction: {:?}", e))
						}).unwrap();

						//TODO: Return emptiness for now, should be a proper enum return type
						Ok(CbData {
							output: String::from(""),
							kernel: String::from(""),
							derivation: 0,
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
	keychain: &Keychain,
	fees: u64,
	mut derivation: u32,
) -> Result<(Output, TxKernel, u32), Error> {
	let fingerprint = keychain.clone().fingerprint();

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		if derivation == 0 {
			derivation = wallet_data.next_child(fingerprint.clone());
		}
		let pubkey = keychain.derive_pubkey(derivation)?;

		// track the new output and return the stuff needed for reward
		wallet_data.add_output(OutputData {
			fingerprint: fingerprint.clone(),
			identifier: pubkey.clone(),
			n_child: derivation,
			value: reward(fees),
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});
		debug!("Received coinbase and built output - {}, {}, {}",
			fingerprint.clone(), pubkey.fingerprint(), derivation);

		let (out, kern) = Block::reward_output(&keychain, pubkey, fees)?;
		Ok((out, kern, derivation))
	})?
}

/// Builds a full transaction from the partial one sent to us for transfer
fn receive_transaction(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	blinding: BlindingFactor,
	partial: Transaction,
) -> Result<Transaction, Error> {

	let fingerprint = keychain.clone().fingerprint();

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let derivation = wallet_data.next_child(fingerprint.clone());
		let pubkey = keychain.derive_pubkey(derivation)?;

		// TODO - replace with real fee calculation
		// TODO - note we are not enforcing this in consensus anywhere yet
		// Note: consensus rules require this to be an even value so it can be split
		let fee_amount = 10;
		let out_amount = amount - fee_amount;

		let (tx_final, _) = build::transaction(vec![
			build::initial_tx(partial),
			build::with_excess(blinding),
			build::output(out_amount, pubkey.clone()),
			build::with_fee(fee_amount),
		], keychain)?;

		// make sure the resulting transaction is valid (could have been lied to on excess)
		tx_final.validate(&keychain.secp())?;

		// track the new output and return the finalized transaction to broadcast
		wallet_data.add_output(OutputData {
			fingerprint: fingerprint.clone(),
			identifier: pubkey.clone(),
			n_child: derivation,
			value: out_amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});
		debug!("Received txn and built output  - {}, {}, {}",
			fingerprint.clone(), pubkey.fingerprint(), derivation);

		Ok(tx_final)
	})?
}
