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

use uuid::Uuid;

use api;
use client;
use checker;
use core::core::amount_to_hr_string;
use libwallet::{aggsig, build, transaction};
use grinwallet::selection;
use core::ser;
use keychain::{Identifier, Keychain};
use receiver::TxWrapper;
use types::*;
use util::LOGGER;
use util;
use failure::ResultExt;

/// Issue a new transaction to the provided sender by spending some of our
/// wallet
/// Outputs. The destination can be "stdout" (for command line) (currently disabled) or a URL to the
/// recipients wallet receiver (to be implemented).
pub fn issue_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	minimum_confirmations: u64,
	dest: String,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
	fluff: bool,
) -> Result<(), Error> {
	checker::refresh_outputs(config, keychain)?;

	// Create a new aggsig context
	let mut context_manager = aggsig::ContextManager::new();
	let tx_id = Uuid::new_v4();

	// Get lock height
	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;
	// ensure outputs we're selecting are up to date
	checker::refresh_outputs(config, keychain)?;

	let lock_height = current_height;

	let tx_data = selection::build_send_tx(
		config,
		keychain,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
		max_outputs,
		selection_strategy_is_use_all,
	)?;

	let partial_tx = transaction::sender_initiation(
		keychain,
		&tx_id,
		&mut context_manager,
		current_height,
		tx_data,
	)?;

	let context = context_manager.get_context(&tx_id);

	// Closure to acquire wallet lock and lock the coins being spent
	// so we avoid accidental double spend attempt.
	let update_wallet = || {
		WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
			for id in context.get_outputs().clone() {
				let coin = wallet_data.get_output(&id).unwrap().clone();
				wallet_data.lock_output(&coin);
			}
		})
	};

	// Closure to acquire wallet lock and delete the change output in case of tx
	// failure.
	let rollback_wallet = || {
		WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
			match context.change_key.clone() {
				Some(change) => {
					info!(LOGGER, "cleaning up unused change output from wallet");
					wallet_data.delete_output(&change);
				}
				None => info!(LOGGER, "No change output to clean from wallet"),
			}
		})
	};

	// TODO: stdout option removed for now, as it won't work very will with this
	// version of aggsig exchange

	/*if dest == "stdout" {
		let json_tx = serde_json::to_string_pretty(&partial_tx).unwrap();
		update_wallet()?;
		println!("{}", json_tx);
	} else */

	if &dest[..4] != "http" {
		WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
			match context.change_key.clone() {
				Some(change) => {
					info!(LOGGER, "cleaning up unused change output from wallet");
					wallet_data.delete_output(&change);
				}
				None => info!(LOGGER, "No change output to clean from wallet"),
			}
		}).unwrap();
		panic!(
			"dest formatted as {} but send -d expected stdout or http://IP:port",
			dest
		);
	}

	let url = format!("{}/v1/receive/transaction", &dest);
	debug!(LOGGER, "Posting partial transaction to {}", url);
	let res = client::send_partial_tx(&url, &partial_tx, fluff);
	if let Err(e) = res {
		match e.kind() {
			ErrorKind::FeeExceedsAmount {
				sender_amount,
				recipient_fee,
			} => error!(
					LOGGER,
					"Recipient rejected the transfer because transaction fee ({}) exceeded amount ({}).",
					amount_to_hr_string(recipient_fee),
					amount_to_hr_string(sender_amount)
				),
			_ => error!(
				LOGGER,
				"Communication with receiver failed on SenderInitiation send. Aborting transaction"
			),
		}
		rollback_wallet()?;
		return Err(e);
	}

	let partial_tx =
		transaction::sender_confirmation(keychain, &mut context_manager, res.unwrap())?;

	// And send again, expecting completed transaction as result this time
	let res = client::send_partial_tx_final(&url, &partial_tx, fluff);
	if let Err(e) = res {
		match e.kind() {
			ErrorKind::FeeExceedsAmount {sender_amount, recipient_fee} =>
				error!(
					LOGGER,
					"Recipient rejected the transfer because transaction fee ({}) exceeded amount ({}).",
					amount_to_hr_string(recipient_fee),
					amount_to_hr_string(sender_amount)
				),
			_ => error!(LOGGER, "Communication with receiver failed on SenderConfirmation send. Aborting transaction"),
		}
		rollback_wallet()?;
		return Err(e);
	}

	// Not really necessary here
	context_manager.save_context(context.clone());

	// All good so
	update_wallet()?;
	Ok(())
}

pub fn issue_burn_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	minimum_confirmations: u64,
	max_outputs: usize,
) -> Result<(), Error> {
	let keychain = &Keychain::burn_enabled(keychain, &Identifier::zero());

	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;

	let _ = checker::refresh_outputs(config, keychain);

	let key_id = keychain.root_key_id();

	// select some spendable coins from the wallet
	let coins = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		Ok(wallet_data.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			false,
		))
	})?;

	debug!(LOGGER, "selected some coins - {}", coins.len());

	let fee = tx_fee(coins.len(), 2, selection::coins_proof_count(&coins), None);
	let (mut parts, _) = selection::inputs_and_change(&coins, config, keychain, amount, fee)?;

	// add burn output and fees
	parts.push(build::output(amount - fee, Identifier::zero()));

	// finalize the burn transaction and send
	let tx_burn = build::transaction(parts, &keychain).context(ErrorKind::Keychain)?;
	tx_burn.validate().context(ErrorKind::Transaction)?;

	let tx_hex = util::to_hex(ser::ser_vec(&tx_burn).unwrap());
	let url = format!("{}/v1/pool/push", config.check_node_api_http_addr.as_str());
	let _: () =
		api::client::post(url.as_str(), &TxWrapper { tx_hex: tx_hex }).context(ErrorKind::Node)?;
	Ok(())
}

#[cfg(test)]
mod test {
	use libwallet::build;
	use keychain::Keychain;

	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the public key and amount begin spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();

		let tx1 = build::transaction(vec![build::output(105, key_id1.clone())], &keychain).unwrap();
		let tx2 = build::transaction(vec![build::input(105, key_id1.clone())], &keychain).unwrap();

		assert_eq!(tx1.outputs[0].features, tx2.inputs[0].features);
		assert_eq!(tx1.outputs[0].commitment(), tx2.inputs[0].commitment());
	}
}
