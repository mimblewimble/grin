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

//! Selection of inputs for building transactions

use core::core::{amount_to_hr_string, Committed, Transaction};
use libwallet::{aggsig, build};
use keychain::{BlindSum, BlindingFactor, Identifier, Keychain};
use types::*;
use util::{secp, LOGGER};
use util::secp::key::{PublicKey, SecretKey};
use util::secp::Signature;
use failure::ResultExt;

/// Builds a transaction to send to someone from the HD seed associated with the
/// wallet and the amount to send. Handles reading through the wallet data file,
/// selecting outputs to spend and building the change.
pub fn build_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	current_height: u64,
	minimum_confirmations: u64,
	lock_height: u64,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<
	(
		Transaction,
		BlindingFactor,
		Vec<OutputData>,
		Option<Identifier>,
		u64,
	),
	Error,
> {
	let key_id = keychain.clone().root_key_id();

	// select some spendable coins from the wallet
	let mut coins = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		Ok(wallet_data.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			selection_strategy_is_use_all,
		))
	})?;

	// Get the maximum number of outputs in the wallet
	let max_outputs = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		Ok(wallet_data.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			true,
		))
	})?.len();

	// sender is responsible for setting the fee on the partial tx
	// recipient should double check the fee calculation and not blindly trust the
	// sender
	let mut fee;
	// First attempt to spend without change
	fee = tx_fee(coins.len(), 1, coins_proof_count(&coins), None);
	let mut total: u64 = coins.iter().map(|c| c.value).sum();
	let mut amount_with_fee = amount + fee;

	if total == 0 {
		return Err(ErrorKind::NotEnoughFunds(total as u64))?;
	}

	// Check if we need to use a change address
	if total > amount_with_fee {
		fee = tx_fee(coins.len(), 2, coins_proof_count(&coins), None);
		amount_with_fee = amount + fee;

		// Here check if we have enough outputs for the amount including fee otherwise
		// look for other outputs and check again
		while total < amount_with_fee {
			// End the loop if we have selected all the outputs and still not enough funds
			if coins.len() == max_outputs {
				return Err(ErrorKind::NotEnoughFunds(total as u64))?;
			}

			// select some spendable coins from the wallet
			coins = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
				Ok(wallet_data.select_coins(
					key_id.clone(),
					amount_with_fee,
					current_height,
					minimum_confirmations,
					max_outputs,
					selection_strategy_is_use_all,
				))
			})?;
			fee = tx_fee(coins.len(), 2, coins_proof_count(&coins), None);
			total = coins.iter().map(|c| c.value).sum();
			amount_with_fee = amount + fee;
		}
	}

	// build transaction skeleton with inputs and change
	let (mut parts, change_key) = inputs_and_change(&coins, config, keychain, amount, fee)?;

	// This is more proof of concept than anything but here we set lock_height
	// on tx being sent (based on current chain height via api).
	parts.push(build::with_lock_height(lock_height));

	let (tx, blind) = build::partial_transaction(parts, &keychain).context(ErrorKind::Keychain)?;

	Ok((tx, blind, coins, change_key, amount_with_fee))
}

/// coins proof count
pub fn coins_proof_count(coins: &Vec<OutputData>) -> usize {
	coins.iter().filter(|c| c.merkle_proof.is_some()).count()
}

/// Selects inputs and change for a transaction
pub fn inputs_and_change(
	coins: &Vec<OutputData>,
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	fee: u64,
) -> Result<(Vec<Box<build::Append>>, Option<Identifier>), Error> {
	let mut parts = vec![];

	// calculate the total across all inputs, and how much is left
	let total: u64 = coins.iter().map(|c| c.value).sum();

	parts.push(build::with_fee(fee));

	// if we are spending 10,000 coins to send 1,000 then our change will be 9,000
	// if the fee is 80 then the recipient will receive 1000 and our change will be
	// 8,920
	let change = total - amount - fee;

	// build inputs using the appropriate derived key_ids
	for coin in coins {
		let key_id = keychain
			.derive_key_id(coin.n_child)
			.context(ErrorKind::Keychain)?;
		if coin.is_coinbase {
			let block = coin.block.clone();
			let merkle_proof = coin.merkle_proof.clone();
			let merkle_proof = merkle_proof.unwrap().merkle_proof();

			parts.push(build::coinbase_input(
				coin.value,
				block.unwrap().hash(),
				merkle_proof,
				key_id,
			));
		} else {
			parts.push(build::input(coin.value, key_id));
		}
	}
	let change_key;
	if change != 0 {
		// track the output representing our change
		change_key = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
			let root_key_id = keychain.root_key_id();
			let change_derivation = wallet_data.next_child(root_key_id.clone());
			let change_key = keychain.derive_key_id(change_derivation).unwrap();

			wallet_data.add_output(OutputData {
				root_key_id: root_key_id.clone(),
				key_id: change_key.clone(),
				n_child: change_derivation,
				value: change as u64,
				status: OutputStatus::Unconfirmed,
				height: 0,
				lock_height: 0,
				is_coinbase: false,
				block: None,
				merkle_proof: None,
			});

			Some(change_key)
		})?;

		parts.push(build::output(change, change_key.clone().unwrap()));
	} else {
		change_key = None
	}

	Ok((parts, change_key))
}
