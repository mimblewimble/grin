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

use serde_json;

use api;
use client;
use checker;
use core::core::{build, Transaction, amount_to_hr_string};
use core::ser;
use keychain::{BlindingFactor, Identifier, Keychain};
use receiver::TxWrapper;
use types::*;
use util::LOGGER;
use util;

/// Issue a new transaction to the provided sender by spending some of our
/// wallet
/// UTXOs. The destination can be "stdout" (for command line) or a URL to the
/// recipients wallet receiver (to be implemented).

pub fn issue_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	minimum_confirmations: u64,
	dest: String,
) -> Result<(), Error> {
	checker::refresh_outputs(config, keychain)?;

	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;

	// proof of concept - set lock_height on the tx
	let lock_height = chain_tip.height;

	let (tx, blind_sum) = build_send_tx(
		config,
		keychain,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
	)?;

	let partial_tx = build_partial_tx(amount, blind_sum, tx);

	if dest == "stdout" {
		let json_tx = serde_json::to_string_pretty(&partial_tx).unwrap();
		println!("{}", json_tx);
	} else if &dest[..4] == "http" {
		let url = format!("{}/v1/receive/transaction", &dest);
		debug!(LOGGER, "Posting partial transaction to {}", url);
		client::send_partial_tx(&url, &partial_tx)?;
	} else {
		panic!("dest not in expected format: {}", dest);
	}
	Ok(())
}

/// Builds a transaction to send to someone from the HD seed associated with the
/// wallet and the amount to send. Handles reading through the wallet data file,
/// selecting outputs to spend and building the change.
fn build_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	current_height: u64,
	minimum_confirmations: u64,
	lock_height: u64,
) -> Result<(Transaction, BlindingFactor), Error> {
	let key_id = keychain.clone().root_key_id();

	// select some spendable coins from the wallet
	let coins = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		wallet_data.select(key_id.clone(), current_height, minimum_confirmations)
	})?;

	// build transaction skeleton with inputs and change
	let parts = inputs_and_change(&coins, config, keychain, key_id, amount);

	if let Err(p) = parts {
		let total: u64 = coins.iter().map(|c| c.value).sum();
		error!(LOGGER, "Transaction not sent - Not enough funds (Max: {})", amount_to_hr_string(total));
		return Err(p);
	}

	let mut parts=parts.unwrap();

	// This is more proof of concept than anything but here we set lock_height
	// on tx being sent (based on current chain height via api).
	parts.push(build::with_lock_height(lock_height));

	let (tx, blind) = build::transaction(parts, &keychain)?;

	Ok((tx, blind))
}

pub fn issue_burn_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	minimum_confirmations: u64,
) -> Result<(), Error> {
	let keychain = &Keychain::burn_enabled(keychain, &Identifier::zero());

	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;

	let _ = checker::refresh_outputs(config, keychain);

	let key_id = keychain.root_key_id();

	// select some spendable coins from the wallet
	let coins = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		wallet_data.select(key_id.clone(), current_height, minimum_confirmations)
	})?;

	let mut parts = inputs_and_change(&coins, config, keychain, key_id, amount)?;

	// add burn output and fees
	let fee = tx_fee(coins.len(), 2, None);
	parts.push(build::output(amount - fee, Identifier::zero()));

	// finalize the burn transaction and send
	let (tx_burn, _) = build::transaction(parts, &keychain)?;
	tx_burn.validate()?;

	let tx_hex = util::to_hex(ser::ser_vec(&tx_burn).unwrap());
	let url = format!("{}/v1/pool/push", config.check_node_api_http_addr.as_str());
	let _: () =
		api::client::post(url.as_str(), &TxWrapper { tx_hex: tx_hex }).map_err(|e| Error::Node(e))?;
	Ok(())
}

fn next_available_key(
	config: &WalletConfig,
	keychain: &Keychain,
) -> Result<(Identifier, u32), Error> {
	let res = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		let root_key_id = keychain.root_key_id();
		let derivation = wallet_data.next_child(root_key_id.clone());
		let key_id = keychain.derive_key_id(derivation).unwrap();
		(key_id, derivation)
	})?;
	Ok(res)
}

fn inputs_and_change(
	coins: &Vec<OutputData>,
	config: &WalletConfig,
	keychain: &Keychain,
	root_key_id: Identifier,
	amount: u64,
) -> Result<Vec<Box<build::Append>>, Error> {
	let mut parts = vec![];

	// calculate the total across all inputs, and how much is left
	let total: u64 = coins.iter().map(|c| c.value).sum();
	let shortage = (total as i64) - (amount as i64);
	if shortage < 0 {
		return Err(Error::NotEnoughFunds((-shortage) as u64));
	}

	// sender is responsible for setting the fee on the partial tx
 // recipient should double check the fee calculation and not blindly trust the
 // sender
	let fee = tx_fee(coins.len(), 2, None);
	parts.push(build::with_fee(fee));

	// if we are spending 10,000 coins to send 1,000 then our change will be 9,000
 // the fee will come out of the amount itself
 // if the fee is 80 then the recipient will only receive 920
 // but our change will still be 9,000
	let change = total - amount;

	// build inputs using the appropriate derived key_ids
	for coin in coins {
		let key_id = keychain.derive_key_id(coin.n_child)?;
		parts.push(build::input(coin.value, key_id));
	}

	let (change_key, change_derivation) = next_available_key(config, keychain)?;

	parts.push(build::output(change, change_key.clone()));

	// Acquire wallet lock, add the new change output and lock coins being spent.
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		// we got that far, time to start tracking the output representing our change
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: change_key.clone(),
			n_child: change_derivation,
			value: change as u64,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
		});

		// now lock the ouputs we're spending so we avoid accidental double spend
		// attempt
		for coin in coins {
			wallet_data.lock_output(coin);
		}
	})?;

	Ok(parts)
}

#[cfg(test)]
mod test {
	use core::core::build::{input, output, transaction};
	use keychain::Keychain;

	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the public key and amount begin spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();

		let (tx1, _) = transaction(vec![output(105, key_id1.clone())], &keychain).unwrap();
		let (tx2, _) = transaction(vec![input(105, key_id1.clone())], &keychain).unwrap();

		assert_eq!(tx1.outputs[0].commitment(), tx2.inputs[0].commitment());
	}
}
