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

//! Provides the JSON/HTTP API for wallets to receive payments. Because
//! receiving money in MimbleWimble requires an interactive exchange, a
//! wallet server that's running at all time is required in many cases.

use bodyparser;
use iron::prelude::*;
use iron::Handler;
use iron::status;
use serde_json;

use api;
use core::consensus::reward;
use core::core::{build, Block, Output, Transaction, TxKernel};
use core::ser;
use keychain::{BlindingFactor, Identifier, Keychain};
use types::*;
use util;
use util::LOGGER;

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	pub tx_hex: String,
}

pub fn receive_json_tx_str(
	config: &WalletConfig,
	keychain: &Keychain,
	json_tx: &str,
) -> Result<(), Error> {
	let partial_tx = serde_json::from_str(json_tx).unwrap();
	receive_json_tx(config, keychain, &partial_tx)
}

/// Receive an already well formed JSON transaction issuance and finalize the
/// transaction, adding our receiving output, to broadcast to the rest of the
/// network.
pub fn receive_json_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	partial_tx: &PartialTx,
) -> Result<(), Error> {
	let (amount, blinding, tx) = read_partial_tx(keychain, partial_tx)?;
	let final_tx = receive_transaction(config, keychain, amount, blinding, tx)?;
	let tx_hex = util::to_hex(ser::ser_vec(&final_tx).unwrap());

	let url = format!("{}/v1/pool/push", config.check_node_api_http_addr.as_str());
	api::client::post(url.as_str(), &TxWrapper { tx_hex: tx_hex })
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

impl Handler for WalletReceiver {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let struct_body = req.get::<bodyparser::Struct<PartialTx>>();

		if let Ok(Some(partial_tx)) = struct_body {
			receive_json_tx(&self.config, &self.keychain, &partial_tx)
				.map_err(|e| {
					api::Error::Internal(
						format!("Error processing partial transaction: {:?}", e),
					)})
				.unwrap();
			Ok(Response::with(status::Ok))
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

fn retrieve_existing_key(
	wallet_data: &WalletData,
	key_id: Identifier,
) -> (Identifier, u32) {
	if let Some(existing) = wallet_data.get_output(&key_id) {
		let key_id = existing.key_id.clone();
		let derivation = existing.n_child;
		(key_id, derivation)
	} else {
		panic!("should never happen");
	}
}

fn next_available_key(
	wallet_data: &WalletData,
	keychain: &Keychain,
) -> (Identifier, u32) {
	let root_key_id = keychain.root_key_id();
	let derivation = wallet_data.next_child(root_key_id.clone());
	let key_id = keychain.derive_key_id(derivation).unwrap();
	(key_id, derivation)
}

/// Build a coinbase output and the corresponding kernel
pub fn receive_coinbase(
	config: &WalletConfig,
	keychain: &Keychain,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error> {
	let root_key_id = keychain.root_key_id();

	// Now acquire the wallet lock and write the new output.
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let key_id = block_fees.key_id();
		let (key_id, derivation) = match key_id {
			Some(key_id) => retrieve_existing_key(&wallet_data, key_id),
			None => next_available_key(&wallet_data, keychain),
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
			is_coinbase: true,
		});

		(key_id, derivation)
	})?;

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

	let (out, kern) = Block::reward_output(&keychain, &key_id, block_fees.fees)?;
	Ok((out, kern, block_fees))
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

	// operate within a lock on wallet data
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let (key_id, derivation) = next_available_key(&wallet_data, keychain);

		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: out_amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
		});

		(key_id, derivation)
	})?;

	let (tx_final, _) = build::transaction(
		vec![
			build::initial_tx(partial),
			build::with_excess(blinding),
			build::output(out_amount, key_id.clone()),
		// build::with_fee(fee_amount),
		],
		keychain,
	)?;

	// make sure the resulting transaction is valid (could have been lied to on
 // excess).
	tx_final.validate()?;

	debug!(
		LOGGER,
		"Received txn and built output - {:?}, {:?}, {}",
		root_key_id.clone(),
		key_id.clone(),
		derivation,
	);

	Ok(tx_final)
}
