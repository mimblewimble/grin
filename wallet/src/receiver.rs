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
use keychain::{BlindingFactor, Keychain};
use types::*;
use util;
use util::LOGGER;

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	pub tx_hex: String,
}

/// Receive an already well formed JSON transaction issuance and finalize the
/// transaction, adding our receiving output, to broadcast to the rest of the
/// network.
pub fn receive_json_tx(config: &WalletConfig,
                       keychain: &Keychain,
                       partial_tx_str: &str)
                       -> Result<(), Error> {
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
		vec![Operation::Custom("coinbase".to_string()),
		     Operation::Custom("receive_json_tx".to_string())]
	}

	fn operation(&self, op: String, input: WalletReceiveRequest) -> ApiResult<CbData> {
		match op.as_str() {
			"coinbase" => {
				match input {
					WalletReceiveRequest::Coinbase(cb_fees) => {
						debug!(LOGGER, "Operation {} with fees {:?}", op, cb_fees);
						let (out, kern, block_fees) = receive_coinbase(
							&self.config,
							&self.keychain,
							&cb_fees,
						).map_err(|e| {
							api::Error::Internal(format!("Error building coinbase: {:?}", e))
						})?;
						let out_bin = ser::ser_vec(&out).map_err(|e| {
							api::Error::Internal(format!("Error serializing output: {:?}", e))
						})?;
						let kern_bin = ser::ser_vec(&kern).map_err(|e| {
							api::Error::Internal(format!("Error serializing kernel: {:?}", e))
						})?;
						let key_id_bin = match block_fees.key_id {
							Some(key_id) => {
								ser::ser_vec(&key_id).map_err(|e| {
									api::Error::Internal(
										format!("Error serializing kernel: {:?}", e),
									)
								})?
							}
							None => vec![],
						};

						Ok(CbData {
							output: util::to_hex(out_bin),
							kernel: util::to_hex(kern_bin),
							key_id: util::to_hex(key_id_bin),
						})
					}
					_ => Err(api::Error::Argument(format!("Incorrect request data: {}", op))),
				}
			}
			"receive_json_tx" => {
				match input {
					WalletReceiveRequest::PartialTransaction(partial_tx_str) => {
						debug!(
							LOGGER,
							"Operation {} with transaction {}",
							op,
							&partial_tx_str,
						);
						receive_json_tx(&self.config, &self.keychain, &partial_tx_str)
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
							key_id: String::from(""),
						})
					}
					_ => Err(api::Error::Argument(format!("Incorrect request data: {}", op))),
				}
			}
			_ => Err(api::Error::Argument(format!("Unknown operation: {}", op))),
		}
	}
}

/// Build a coinbase output and the corresponding kernel
fn receive_coinbase(config: &WalletConfig,
                    keychain: &Keychain,
                    block_fees: &BlockFees)
                    -> Result<(Output, TxKernel, BlockFees), Error> {
	let root_key_id = keychain.root_key_id();

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let key_id = block_fees.key_id();
		let (key_id, derivation) = match key_id {
			Some(key_id) => {
				let derivation = keychain.derivation_from_key_id(&key_id)?;
				(key_id.clone(), derivation)
			},
			None => {
				let derivation = wallet_data.next_child(root_key_id.clone());
				let key_id = keychain.derive_key_id(derivation)?;
				(key_id, derivation)
			}
		};

		// track the new output and return the stuff needed for reward
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: reward(block_fees.fees),
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});

		debug!(
			LOGGER,
			"Received coinbase and built candidate output - {:?}, {:?}, {}",
			root_key_id.clone(),
			key_id.clone(),
			derivation,
		);

		debug!(LOGGER, "block_fees - {:?}", block_fees);

		let mut block_fees = block_fees.clone();
		block_fees.key_id = Some(key_id.clone());

		debug!(LOGGER, "block_fees updated - {:?}", block_fees);

		let (out, kern) = Block::reward_output(
			&keychain,
			&key_id,
			block_fees.fees,
		)?;
		Ok((out, kern, block_fees))
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
	let root_key_id = keychain.root_key_id();

	// operate within a lock on wallet data
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let derivation = wallet_data.next_child(root_key_id.clone());
		let key_id = keychain.derive_key_id(derivation)?;

		// double check the fee amount included in the partial tx
		// we don't necessarily want to just trust the sender
		// we could just overwrite the fee here (but we won't) due to the ecdsa sig
		let fee = tx_fee(partial.inputs.len(), partial.outputs.len() + 1, None);
		if fee != partial.fee {
			return Err(Error::FeeDispute {
				sender_fee: partial.fee,
				recipient_fee: fee,
			});
		}

		let out_amount = amount - fee;

		let (tx_final, _) = build::transaction(vec![
			build::initial_tx(partial),
			build::with_excess(blinding),
			build::output(out_amount, key_id.clone()),
			// build::with_fee(fee_amount),
		], keychain)?;

		// make sure the resulting transaction is valid (could have been lied to on
		// excess)
		tx_final.validate(&keychain.secp())?;

		// track the new output and return the finalized transaction to broadcast
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: out_amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
		});
		debug!(
			LOGGER,
			"Received txn and built output - {:?}, {:?}, {}",
			root_key_id.clone(),
			key_id.clone(),
			derivation,
		);

		Ok(tx_final)
	})?
}
