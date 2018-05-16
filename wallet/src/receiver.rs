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

//! Provides the JSON/HTTP API for wallets to receive payments. Because
//! receiving money in MimbleWimble requires an interactive exchange, a
//! wallet server that's running at all time is required in many cases.

use bodyparser;
use iron::Handler;
use iron::prelude::*;
use iron::status;
use serde_json;
use std::sync::{Arc, RwLock};

use api;
use core::consensus::reward;
use core::core::{Output, Transaction, TxKernel};
use libwallet::{aggsig, reward, transaction};
use grinwallet::keys;
use core::{global, ser};
use failure::{Fail, ResultExt};
use keychain::Keychain;
use types::*;
use urlencoded::UrlEncodedQuery;
use util::{to_hex, LOGGER};

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	pub tx_hex: String,
}

lazy_static! {
	/// Static reference to aggsig context (temporary while wallet is being refactored)
	pub static ref AGGSIG_CONTEXT_MANAGER:Arc<RwLock<aggsig::ContextManager>>
		= Arc::new(RwLock::new(aggsig::ContextManager::new()));
}

fn handle_sender_initiation(
	config: &WalletConfig,
	context_manager: &mut aggsig::ContextManager,
	keychain: &Keychain,
	partial_tx: &PartialTx,
) -> Result<PartialTx, Error> {
	// Create a potential output for this transaction
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		keys::next_available_key(&wallet_data, keychain)
	})?;

	let partial_tx =
		transaction::recipient_initiation(keychain, context_manager, partial_tx, &key_id)?;
	let mut context = context_manager.get_context(&partial_tx.id);
	context.add_output(&key_id);

	// Add the output to our wallet
	let _ = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		wallet_data.add_output(OutputData {
			root_key_id: keychain.root_key_id(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: partial_tx.amount - context.fee,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
			block: None,
			merkle_proof: None,
		});
	})?;

	context_manager.save_context(context);
	Ok(partial_tx)
}

fn handle_sender_confirmation(
	config: &WalletConfig,
	context_manager: &mut aggsig::ContextManager,
	keychain: &Keychain,
	partial_tx: &PartialTx,
	fluff: bool,
) -> Result<Transaction, Error> {
	let context = context_manager.get_context(&partial_tx.id);
	// Get output we created in earlier step
	// TODO: will just be one for now, support multiple later
	let output_vec = context.get_outputs();

	let root_key_id = keychain.root_key_id();
	// operate within a lock on wallet data
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let (key_id, derivation) = keys::retrieve_existing_key(&wallet_data, output_vec[0].clone());

		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: partial_tx.amount - context.fee,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
			block: None,
			merkle_proof: None,
		});

		(key_id, derivation)
	})?;

	// In this case partial_tx contains other party's pubkey info
	let final_tx = transaction::finalize_transaction(
		keychain,
		context_manager,
		partial_tx,
		partial_tx,
		&key_id,
		derivation,
	)?;

	let tx_hex = to_hex(ser::ser_vec(&final_tx).unwrap());

	let url;
	if fluff {
		url = format!(
			"{}/v1/pool/push?fluff",
			config.check_node_api_http_addr.as_str()
		);
	} else {
		url = format!("{}/v1/pool/push", config.check_node_api_http_addr.as_str());
	}
	api::client::post(url.as_str(), &TxWrapper { tx_hex: tx_hex }).context(ErrorKind::Node)?;

	Ok(final_tx)
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

		let mut fluff = false;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(_) = params.get("fluff") {
				fluff = true;
			}
		}

		if let Ok(Some(partial_tx)) = struct_body {
			let mut acm = AGGSIG_CONTEXT_MANAGER.write().unwrap();
			match partial_tx.phase {
				PartialTxPhase::SenderInitiation => {
					let resp_tx = handle_sender_initiation(
						&self.config,
						&mut acm,
						&self.keychain,
						&partial_tx,
					).map_err(|e| {
						error!(LOGGER, "Phase 1 Sender Initiation -> Problematic partial tx, looks like this: {:?}", partial_tx);
						e.context(api::ErrorKind::Internal(
							"Error processing partial transaction".to_owned(),
						))
					})
						.unwrap();
					let json = serde_json::to_string(&resp_tx).unwrap();
					Ok(Response::with((status::Ok, json)))
				}
				PartialTxPhase::SenderConfirmation => {
					let resp_tx = handle_sender_confirmation(
						&self.config,
						&mut acm,
						&self.keychain,
						&partial_tx,
						fluff,
					).map_err(|e| {
						error!(LOGGER, "Phase 3 Sender Confirmation -> Problematic partial tx, looks like this: {:?}", partial_tx);
						e.context(api::ErrorKind::Internal(
							"Error processing partial transaction".to_owned(),
						))
					})
						.unwrap();
					let json = serde_json::to_string(&resp_tx).unwrap();
					Ok(Response::with((status::Ok, json)))
				}
				_ => {
					error!(LOGGER, "Unhandled Phase: {:?}", partial_tx);
					Ok(Response::with((status::BadRequest, "Unhandled Phase")))
				}
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

//TODO: Split up the output creation and the wallet insertion
/// Build a coinbase output and the corresponding kernel
pub fn receive_coinbase(
	config: &WalletConfig,
	keychain: &Keychain,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error> {
	let root_key_id = keychain.root_key_id();

	let height = block_fees.height;
	let lock_height = height + global::coinbase_maturity();

	// Now acquire the wallet lock and write the new output.
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let key_id = block_fees.key_id();
		let (key_id, derivation) = match key_id {
			Some(key_id) => keys::retrieve_existing_key(&wallet_data, key_id),
			None => keys::next_available_key(&wallet_data, keychain),
		};

		// track the new output and return the stuff needed for reward
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: reward(block_fees.fees),
			status: OutputStatus::Unconfirmed,
			height: height,
			lock_height: lock_height,
			is_coinbase: true,
			block: None,
			merkle_proof: None,
		});

		(key_id, derivation)
	})?;

	debug!(
		LOGGER,
		"receive_coinbase: built candidate output - {:?}, {}",
		key_id.clone(),
		derivation,
	);

	let mut block_fees = block_fees.clone();
	block_fees.key_id = Some(key_id.clone());

	debug!(LOGGER, "receive_coinbase: {:?}", block_fees);

	let (out, kern) =
		reward::output(&keychain, &key_id, block_fees.fees, block_fees.height).unwrap();
	/* .context(ErrorKind::Keychain)?; */
	Ok((out, kern, block_fees))
}
