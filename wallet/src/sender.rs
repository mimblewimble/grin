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

use api;
use checker;
use client;
use core::core::amount_to_hr_string;
use core::ser;
use failure::ResultExt;
use grinwallet::selection;
use keychain::{Identifier, Keychain};
use libwallet::{aggsig, build};
use receiver::TxWrapper;
use types::*;
use util;
use util::LOGGER;

/// Issue a new transaction to the provided sender by spending some of our
/// wallet
/// Outputs. The destination can be "stdout" (for command line) (currently
/// disabled) or a URL to the recipients wallet receiver (to be implemented).
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
	// TODO: Stdout option, probably in a separate implementation
	if &dest[..4] != "http" {
		panic!(
			"dest formatted as {} but send -d expected stdout or http://IP:port",
			dest
		);
	}

	checker::refresh_outputs(config, keychain)?;

	// Create a new aggsig context
	let mut context_manager = aggsig::ContextManager::new();

	// Get lock height
	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;
	// ensure outputs we're selecting are up to date
	checker::refresh_outputs(config, keychain)?;

	let lock_height = current_height;

	// Sender selects outputs into a new slate and save our corresponding IDs in
	// their transaction context. The secret key in our transaction context will be
	// randomly selected. This returns the public slate, and a closure that locks
	// our inputs and outputs once we're convinced the transaction exchange went
	// according to plan
	// This function is just a big helper to do all of that, in theory
	// this process can be split up in any way
	let (mut slate, sender_lock_fn) = selection::build_send_tx_slate(
		config,
		keychain,
		&mut context_manager,
		2,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
		max_outputs,
		selection_strategy_is_use_all,
	).unwrap();

	// Generate a kernel offset and subtract from our context's secret key. Store
	// the offset in the slate's transaction kernel, and adds our public key
	// information to the slate
	let _ = slate
		.fill_round_1(keychain, &mut context_manager, 0)
		.unwrap();

	let url = format!("{}/v1/receive/transaction", &dest);
	debug!(LOGGER, "Posting partial transaction to {}", url);
	let mut slate = match client::send_slate(&url, &slate, fluff) {
		Ok(s) => s,
		Err(e) => {
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
			return Err(e);
		}
	};

	let _ = slate.fill_round_2(keychain, &mut context_manager, 0)?;

	// Final transaction can be built by anyone at this stage
	slate.finalize(keychain)?;

	// So let's post it
	let tx_hex = util::to_hex(ser::ser_vec(&slate.tx).unwrap());
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

	// All good so, lock our outputs
	sender_lock_fn()?;
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
	use keychain::Keychain;
	use libwallet::build;

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
