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

use keychain::{Identifier, Keychain};
use libtx::{build, slate::Slate, tx_fee};
use libwallet::error::{Error, ErrorKind};
use libwallet::internal::{keys, sigcontext};
use libwallet::types::*;

/// Initialize a transaction on the sender side, returns a corresponding
/// libwallet transaction slate with the appropriate inputs selected,
/// and saves the private wallet identifiers of our selected outputs
/// into our transaction context

pub fn build_send_tx_slate<T: ?Sized, C, K>(
	wallet: &mut T,
	num_participants: usize,
	amount: u64,
	current_height: u64,
	minimum_confirmations: u64,
	lock_height: u64,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<
	(
		Slate,
		sigcontext::Context,
		impl FnOnce(&mut T) -> Result<(), Error>,
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let (elems, inputs, change, change_derivation, amount, fee) = select_send_tx(
		wallet,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
		max_outputs,
		selection_strategy_is_use_all,
	)?;

	// Create public slate
	let mut slate = Slate::blank(num_participants);
	slate.amount = amount;
	slate.height = current_height;
	slate.lock_height = lock_height;
	slate.fee = fee;

	let keychain = wallet.keychain().clone();

	let blinding = slate.add_transaction_elements(&keychain, elems)?;
	// Create our own private context
	let mut context = sigcontext::Context::new(
		wallet.keychain().secp(),
		blinding.secret_key(&keychain.secp()).unwrap(),
	);

	// Store our private identifiers for each input
	for input in inputs {
		context.add_input(&input.key_id);
	}

	// Store change output
	if change_derivation.is_some() {
		let change_id = keychain.derive_key_id(change_derivation.unwrap()).unwrap();
		context.add_output(&change_id);
	}

	let lock_inputs = context.get_inputs().clone();
	let _lock_outputs = context.get_outputs().clone();

	let root_key_id = keychain.root_key_id();

	// Return a closure to acquire wallet lock and lock the coins being spent
	// so we avoid accidental double spend attempt.
	let update_sender_wallet_fn = move |wallet: &mut T| {
		let mut batch = wallet.batch()?;
		for id in lock_inputs {
			let mut coin = batch.get(&id).unwrap();
			batch.lock_output(&mut coin)?;
		}
		// write the output representing our change
		if let Some(d) = change_derivation {
			let change_id = keychain.derive_key_id(change_derivation.unwrap()).unwrap();

			batch.save(OutputData {
				root_key_id: root_key_id,
				key_id: change_id.clone(),
				n_child: d,
				value: change as u64,
				status: OutputStatus::Unconfirmed,
				height: current_height,
				lock_height: 0,
				is_coinbase: false,
			})?;
		}
		batch.commit()?;
		Ok(())
	};

	Ok((slate, context, update_sender_wallet_fn))
}

/// Creates a new output in the wallet for the recipient,
/// returning the key of the fresh output and a closure
/// that actually performs the addition of the output to the
/// wallet
pub fn build_recipient_output_with_slate<T: ?Sized, C, K>(
	wallet: &mut T,
	slate: &mut Slate,
) -> Result<
	(
		Identifier,
		sigcontext::Context,
		impl FnOnce(&mut T) -> Result<(), Error>,
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	// Create a potential output for this transaction
	let (key_id, derivation) = keys::next_available_key(wallet).unwrap();

	let keychain = wallet.keychain().clone();
	let root_key_id = keychain.root_key_id();
	let key_id_inner = key_id.clone();
	let amount = slate.amount;
	let height = slate.height;

	let blinding =
		slate.add_transaction_elements(&keychain, vec![build::output(amount, key_id.clone())])?;

	// Add blinding sum to our context
	let mut context = sigcontext::Context::new(
		keychain.secp(),
		blinding
			.secret_key(wallet.keychain().clone().secp())
			.unwrap(),
	);

	context.add_output(&key_id);

	// Create closure that adds the output to recipient's wallet
	// (up to the caller to decide when to do)
	let wallet_add_fn = move |wallet: &mut T| {
		let mut batch = wallet.batch()?;
		batch.save(OutputData {
			root_key_id: root_key_id,
			key_id: key_id_inner,
			n_child: derivation,
			value: amount,
			status: OutputStatus::Unconfirmed,
			height: height,
			lock_height: 0,
			is_coinbase: false,
		})?;
		batch.commit()?;
		Ok(())
	};
	Ok((key_id, context, wallet_add_fn))
}

/// Builds a transaction to send to someone from the HD seed associated with the
/// wallet and the amount to send. Handles reading through the wallet data file,
/// selecting outputs to spend and building the change.
pub fn select_send_tx<T: ?Sized, C, K>(
	wallet: &mut T,
	amount: u64,
	current_height: u64,
	minimum_confirmations: u64,
	lock_height: u64,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<
	(
		Vec<Box<build::Append<K>>>,
		Vec<OutputData>,
		u64,         //change
		Option<u32>, //change derivation
		u64,         // amount
		u64,         // fee
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let key_id = wallet.keychain().root_key_id();

	// select some spendable coins from the wallet
	let mut coins = wallet.select_coins(
		key_id.clone(),
		amount,
		current_height,
		minimum_confirmations,
		max_outputs,
		selection_strategy_is_use_all,
	);

	// Get the maximum number of outputs in the wallet
	let max_outputs = wallet
		.select_coins(
			key_id.clone(),
			amount,
			current_height,
			minimum_confirmations,
			max_outputs,
			true,
		)
		.len();

	// sender is responsible for setting the fee on the partial tx
	// recipient should double check the fee calculation and not blindly trust the
	// sender
	let mut fee;
	// First attempt to spend without change
	fee = tx_fee(coins.len(), 1, None);
	let mut total: u64 = coins.iter().map(|c| c.value).sum();
	let mut amount_with_fee = amount + fee;

	if total == 0 {
		return Err(ErrorKind::NotEnoughFunds {
			available: 0,
			needed: amount_with_fee as u64,
		})?;
	}

	// Check if we need to use a change address
	if total > amount_with_fee {
		fee = tx_fee(coins.len(), 2, None);
		amount_with_fee = amount + fee;

		// Here check if we have enough outputs for the amount including fee otherwise
		// look for other outputs and check again
		while total < amount_with_fee {
			// End the loop if we have selected all the outputs and still not enough funds
			if coins.len() == max_outputs {
				return Err(ErrorKind::NotEnoughFunds {
					available: total as u64,
					needed: amount_with_fee as u64,
				})?;
			}

			// select some spendable coins from the wallet
			coins = wallet.select_coins(
				key_id.clone(),
				amount_with_fee,
				current_height,
				minimum_confirmations,
				max_outputs,
				selection_strategy_is_use_all,
			);
			fee = tx_fee(coins.len(), 2, None);
			total = coins.iter().map(|c| c.value).sum();
			amount_with_fee = amount + fee;
		}
	}

	// build transaction skeleton with inputs and change
	let (mut parts, change, change_derivation) = inputs_and_change(&coins, wallet, amount, fee)?;

	// This is more proof of concept than anything but here we set lock_height
	// on tx being sent (based on current chain height via api).
	parts.push(build::with_lock_height(lock_height));

	Ok((parts, coins, change, change_derivation, amount, fee))
}

/// Selects inputs and change for a transaction
pub fn inputs_and_change<T: ?Sized, C, K>(
	coins: &Vec<OutputData>,
	wallet: &mut T,
	amount: u64,
	fee: u64,
) -> Result<(Vec<Box<build::Append<K>>>, u64, Option<u32>), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
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
		let key_id = wallet.keychain().derive_key_id(coin.n_child)?;
		if coin.is_coinbase {
			parts.push(build::coinbase_input(coin.value, key_id));
		} else {
			parts.push(build::input(coin.value, key_id));
		}
	}
	let mut change_derivation = None;
	if change != 0 {
		let keychain = wallet.keychain().clone();
		let root_key_id = keychain.root_key_id();
		change_derivation = Some(wallet.next_child(root_key_id.clone()).unwrap());
		let change_k = keychain.derive_key_id(change_derivation.unwrap()).unwrap();

		parts.push(build::output(change, change_k.clone()));
	}

	Ok((parts, change, change_derivation))
}
