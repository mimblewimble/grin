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
/// UTXOs. The destination can be "stdout" (for command line) (currently disabled) or a URL to the
/// recipients wallet receiver (to be implemented).
pub fn issue_send_tx(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	minimum_confirmations: u64,
	dest: String,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<(), Error> {
	checker::refresh_outputs(config, keychain)?;

	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;

	// proof of concept - set lock_height on the tx
	let lock_height = chain_tip.height;

	let (tx, blind_sum, coins, change_key) = build_send_tx(
		config,
		keychain,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
		max_outputs,
		selection_strategy_is_use_all,
	)?;
	/*
	 * -Sender picks random blinding factors for all outputs it participates in, computes total blinding excess xS
	 * -Sender picks random nonce kS
	 * -Sender posts inputs, outputs, Message M=fee, xS * G and kS * G to Receiver
	*/

// Create a new aggsig context
	keychain.aggsig_create_context(blind_sum.secret_key());

	let partial_tx = build_partial_tx(keychain, amount, None, tx);

	// Closure to acquire wallet lock and lock the coins being spent
	// so we avoid accidental double spend attempt.
	let update_wallet = || WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		for coin in coins {
			wallet_data.lock_output(&coin);
		}
	});

	// Closure to acquire wallet lock and delete the change output in case of tx failure.
	let rollback_wallet = || WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		info!(LOGGER, "cleaning up unused change output from wallet");
		wallet_data.delete_output(&change_key);
	});

	// TODO: stdout option removed for now, as it won't work very will with this version of
	// aggsig exchange

	/*if dest == "stdout" {
		let json_tx = serde_json::to_string_pretty(&partial_tx).unwrap();
		update_wallet()?;
		println!("{}", json_tx);
	} else */

	if &dest[..4] != "http" {
		panic!("dest formatted as {} but send -d expected stdout or http://IP:port", dest);
	}

	let url = format!("{}/v1/receive/transaction", &dest);
	debug!(LOGGER, "Posting partial transaction to {}", url);
	let res = client::send_partial_tx(&url, &partial_tx);
	if let Err(e) = res {
		match e {
			Error::FeeExceedsAmount {sender_amount, recipient_fee} =>
				error!(
					LOGGER,
					"Recipient rejected the transfer because transaction fee ({}) exceeded amount ({}).",
					amount_to_hr_string(recipient_fee),
					amount_to_hr_string(sender_amount)
				),
			_ => error!(LOGGER, "Communication with receiver failed on SenderInitiation send. Aborting transaction"),
		}
		rollback_wallet()?;
		return Err(e);
	}

	/* -Sender receives xR * G, kR * G, sR
	 * -Sender computes Schnorr challenge e = H(M | kR * G + kS * G)
	 * -Sender verifies receivers sig, by verifying that kR * G + e * xR * G = sR * G·
	 * -Sender computes their part of signature, sS = kS + e * xS
	 * -Sender posts sS to receiver
	*/
	let (_amount, recp_pub_blinding, recp_pub_nonce, sig, tx) = read_partial_tx(keychain, &res.unwrap())?;
	let res = keychain.aggsig_verify_partial_sig(&sig.unwrap(), &recp_pub_nonce, &recp_pub_blinding, tx.fee, lock_height);
	if !res {
		error!(LOGGER, "Partial Sig from recipient invalid.");
		return Err(Error::Signature(String::from("Partial Sig from recipient invalid.")));
	}

	let sig_part=keychain.aggsig_calculate_partial_sig(&recp_pub_nonce, tx.fee, tx.lock_height).unwrap();

	// Build the next stage, containing sS (and our pubkeys again, for the recipient's convenience)
	let mut partial_tx = build_partial_tx(keychain, amount, Some(sig_part), tx);
	partial_tx.phase = PartialTxPhase::SenderConfirmation;

	// And send again
	let res = client::send_partial_tx(&url, &partial_tx);
	if let Err(e) = res {
		match e {
			Error::FeeExceedsAmount {sender_amount, recipient_fee} =>
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
	//All good so
	update_wallet()?;
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
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<(Transaction, BlindingFactor, Vec<OutputData>, Identifier), Error> {
	let key_id = keychain.clone().root_key_id();

	// select some spendable coins from the wallet
	let coins = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		wallet_data.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			selection_strategy_is_use_all,
		)
	})?;

	// build transaction skeleton with inputs and change
	let (mut parts, change_key) = inputs_and_change(&coins, config, keychain, amount)?;

	// This is more proof of concept than anything but here we set lock_height
	// on tx being sent (based on current chain height via api).
	parts.push(build::with_lock_height(lock_height));

	let (tx, blind) = build::transaction(parts, &keychain)?;

	Ok((tx, blind, coins, change_key))
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
		wallet_data.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			false,
		)
	})?;

	debug!(LOGGER, "selected some coins - {}", coins.len());

	let (mut parts, _) = inputs_and_change(&coins, config, keychain, amount)?;

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

fn inputs_and_change(
	coins: &Vec<OutputData>,
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
) -> Result<(Vec<Box<build::Append>>, Identifier), Error> {
	let mut parts = vec![];

	// calculate the total across all inputs, and how much is left
	let total: u64 = coins.iter().map(|c| c.value).sum();
	if total < amount {
		return Err(Error::NotEnoughFunds(total as u64));
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
		if coin.is_coinbase {
			parts.push(build::coinbase_input(coin.value, coin.block.hash(), key_id));
		} else {
			parts.push(build::input(coin.value, coin.block.hash(), key_id));
		}
	}

	// track the output representing our change
	let change_key = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
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
			block: BlockIdentifier::zero(),
		});

		change_key
	})?;

	parts.push(build::output(change, change_key.clone()));

	Ok((parts, change_key))
}

#[cfg(test)]
mod test {
	use core::core::build::{input, output, transaction};
	use core::core::hash::ZERO_HASH;
	use keychain::Keychain;


	#[test]
	// demonstrate that input.commitment == referenced output.commitment
	// based on the public key and amount begin spent
	fn output_commitment_equals_input_commitment_on_spend() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();

		let (tx1, _) = transaction(vec![output(105, key_id1.clone())], &keychain).unwrap();
		let (tx2, _) = transaction(vec![input(105, ZERO_HASH, key_id1.clone())], &keychain).unwrap();

		assert_eq!(tx1.outputs[0].features, tx2.inputs[0].features);
		assert_eq!(tx1.outputs[0].commitment(), tx2.inputs[0].commitment());
	}
}
