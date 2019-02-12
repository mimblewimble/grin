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

use crate::core::core::{amount_to_hr_string, Transaction};
use crate::core::libtx::{build, tx_fee};
use crate::core::{consensus, global};
use crate::keychain::{Identifier, Keychain};
use crate::libwallet::error::{Error, ErrorKind};
use crate::libwallet::internal::keys;
use crate::libwallet::slate::Slate;
use crate::libwallet::types::*;
use std::cmp::min;
use std::collections::HashMap;
use std::marker::PhantomData;

/// Initialize a transaction on the sender side, returns a corresponding
/// libwallet transaction slate with the appropriate inputs selected,
/// and saves the private wallet identifiers of our selected outputs
/// into our transaction context

pub fn build_send_tx<T: ?Sized, C, K>(
	wallet: &mut T,
	slate: &mut Slate,
	minimum_confirmations: u64,
	change_outputs: usize,
	selection_strategy_is_use_all: bool,
	parent_key_id: Identifier,
) -> Result<(Context, OutputLockFn<T, C, K>), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let (elems, inputs, change_amounts_derivations, fee) = select_send_tx(
		wallet,
		slate.amount,
		slate.height,
		minimum_confirmations,
		slate.lock_height,
		change_outputs,
		selection_strategy_is_use_all,
		&parent_key_id,
	)?;

	slate.fee = fee;

	let keychain = wallet.keychain().clone();
	let blinding = slate.add_transaction_elements(&keychain, elems)?;

	// Create our own private context
	let mut context = Context::new(
		wallet.keychain().secp(),
		blinding.secret_key(&keychain.secp()).unwrap(),
	);

	// Store our private identifiers for each input
	for input in inputs {
		context.add_input(&input.key_id, &input.mmr_index);
	}

	let mut commits: HashMap<Identifier, Option<String>> = HashMap::new();

	// Store change output(s) and cached commits
	for (change_amount, id, mmr_index) in &change_amounts_derivations {
		context.add_output(&id, &mmr_index);
		commits.insert(
			id.clone(),
			wallet.calc_commit_for_cache(*change_amount, &id)?,
		);
	}

	let lock_inputs_in = context.get_inputs().clone();
	let _lock_outputs = context.get_outputs().clone();
	let messages_in = Some(slate.participant_messages());
	let slate_id_in = slate.id.clone();
	let height_in = slate.height;

	// Return a closure to acquire wallet lock and lock the coins being spent
	// so we avoid accidental double spend attempt.
	let update_sender_wallet_fn =
		move |wallet: &mut T, tx: &Transaction, _: PhantomData<C>, _: PhantomData<K>| {
			let tx_entry = {
				// These ensure the closure remains FnMut
				let lock_inputs = lock_inputs_in.clone();
				let messages = messages_in.clone();
				let slate_id = slate_id_in.clone();
				let height = height_in.clone();
				let mut batch = wallet.batch()?;
				let log_id = batch.next_tx_log_id(&parent_key_id)?;
				let mut t = TxLogEntry::new(parent_key_id.clone(), TxLogEntryType::TxSent, log_id);
				t.tx_slate_id = Some(slate_id.clone());
				let filename = format!("{}.grintx", slate_id);
				t.stored_tx = Some(filename);
				t.fee = Some(fee);
				let mut amount_debited = 0;
				t.num_inputs = lock_inputs.len();
				for id in lock_inputs {
					let mut coin = batch.get(&id.0, &id.1).unwrap();
					coin.tx_log_entry = Some(log_id);
					amount_debited = amount_debited + coin.value;
					batch.lock_output(&mut coin)?;
				}

				t.amount_debited = amount_debited;
				t.messages = messages;

				// write the output representing our change
				for (change_amount, id, _) in &change_amounts_derivations {
					t.num_outputs += 1;
					t.amount_credited += change_amount;
					let commit = commits.get(&id).unwrap().clone();
					batch.save(OutputData {
						root_key_id: parent_key_id.clone(),
						key_id: id.clone(),
						n_child: id.to_path().last_path_index(),
						commit: commit,
						mmr_index: None,
						value: change_amount.clone(),
						status: OutputStatus::Unconfirmed,
						height: height,
						lock_height: 0,
						is_coinbase: false,
						tx_log_entry: Some(log_id),
					})?;
				}
				batch.save_tx_log_entry(t.clone(), &parent_key_id)?;
				batch.commit()?;
				t
			};
			wallet.store_tx(&format!("{}", tx_entry.tx_slate_id.unwrap()), tx)?;
			Ok(())
		};

	Ok((context, Box::new(update_sender_wallet_fn)))
}

/// Creates a new output in the wallet for the recipient,
/// returning the key of the fresh output and a closure
/// that actually performs the addition of the output to the
/// wallet
pub fn build_recipient_output<T: ?Sized, C, K>(
	wallet: &mut T,
	slate: &mut Slate,
	parent_key_id: Identifier,
) -> Result<(Identifier, Context, OutputLockFn<T, C, K>), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// Create a potential output for this transaction
	let key_id = keys::next_available_key(wallet).unwrap();

	let keychain = wallet.keychain().clone();
	let key_id_inner = key_id.clone();
	let amount = slate.amount;
	let height = slate.height;

	let slate_id = slate.id.clone();
	let blinding =
		slate.add_transaction_elements(&keychain, vec![build::output(amount, key_id.clone())])?;

	// Add blinding sum to our context
	let mut context = Context::new(
		keychain.secp(),
		blinding
			.secret_key(wallet.keychain().clone().secp())
			.unwrap(),
	);

	context.add_output(&key_id, &None);
	let messages_in = Some(slate.participant_messages());

	// Create closure that adds the output to recipient's wallet
	// (up to the caller to decide when to do)
	let wallet_add_fn =
		move |wallet: &mut T, _tx: &Transaction, _: PhantomData<C>, _: PhantomData<K>| {
			// Ensure closure remains FnMut
			let messages = messages_in.clone();
			let commit = wallet.calc_commit_for_cache(amount, &key_id_inner)?;
			let mut batch = wallet.batch()?;
			let log_id = batch.next_tx_log_id(&parent_key_id)?;
			let mut t = TxLogEntry::new(parent_key_id.clone(), TxLogEntryType::TxReceived, log_id);
			t.tx_slate_id = Some(slate_id);
			t.amount_credited = amount;
			t.num_outputs = 1;
			t.messages = messages;
			batch.save(OutputData {
				root_key_id: parent_key_id.clone(),
				key_id: key_id_inner.clone(),
				mmr_index: None,
				n_child: key_id_inner.to_path().last_path_index(),
				commit: commit,
				value: amount,
				status: OutputStatus::Unconfirmed,
				height: height,
				lock_height: 0,
				is_coinbase: false,
				tx_log_entry: Some(log_id),
			})?;
			batch.save_tx_log_entry(t, &parent_key_id)?;
			batch.commit()?;
			//TODO: Check whether we want to call this
			//wallet.store_tx(&format!("{}", t.tx_slate_id.unwrap()), tx)?;
			Ok(())
		};
	Ok((key_id, context, Box::new(wallet_add_fn)))
}

/// Calculate maximal amount of inputs in transaction given amount of outputs
fn calculate_max_inputs_in_block(num_outputs: usize) -> usize {
	let coinbase_weight = consensus::BLOCK_OUTPUT_WEIGHT + consensus::BLOCK_KERNEL_WEIGHT;
	global::max_block_weight().saturating_sub(
		coinbase_weight
			+ consensus::BLOCK_OUTPUT_WEIGHT.saturating_mul(num_outputs)
			+ consensus::BLOCK_KERNEL_WEIGHT,
	) / consensus::BLOCK_INPUT_WEIGHT
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
	change_outputs: usize,
	selection_strategy_is_use_all: bool,
	parent_key_id: &Identifier,
) -> Result<
	(
		Vec<Box<build::Append<K>>>,
		Vec<OutputData>,
		Vec<(u64, Identifier, Option<u64>)>, // change amounts and derivations
		u64,                                 // fee
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let (coins, _total, amount, fee) = select_coins_and_fee(
		wallet,
		amount,
		current_height,
		minimum_confirmations,
		change_outputs,
		selection_strategy_is_use_all,
		&parent_key_id,
	)?;

	// build transaction skeleton with inputs and change
	let (mut parts, change_amounts_derivations) =
		inputs_and_change(&coins, wallet, amount, fee, change_outputs)?;

	// This is more proof of concept than anything but here we set lock_height
	// on tx being sent (based on current chain height via api).
	parts.push(build::with_lock_height(lock_height));

	Ok((parts, coins, change_amounts_derivations, fee))
}

/// Select outputs and calculating fee.
pub fn select_coins_and_fee<T: ?Sized, C, K>(
	wallet: &mut T,
	amount: u64,
	current_height: u64,
	minimum_confirmations: u64,
	change_outputs: usize,
	selection_strategy_is_use_all: bool,
	parent_key_id: &Identifier,
) -> Result<
	(
		Vec<OutputData>,
		u64, // total
		u64, // amount
		u64, // fee
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// select some spendable coins from the wallet
	let (max_outputs, mut coins) = select_coins(
		wallet,
		amount,
		current_height,
		minimum_confirmations,
		calculate_max_inputs_in_block(change_outputs),
		selection_strategy_is_use_all,
		parent_key_id,
	);

	// sender is responsible for setting the fee on the partial tx
	// recipient should double check the fee calculation and not blindly trust the
	// sender

	// TODO - Is it safe to spend without a change output? (1 input -> 1 output)
	// TODO - Does this not potentially reveal the senders private key?
	//
	// First attempt to spend without change
	let mut fee = tx_fee(coins.len(), 1, 1, None);
	let mut total: u64 = coins.iter().map(|c| c.value).sum();
	let mut amount_with_fee = amount + fee;

	if total == 0 {
		return Err(ErrorKind::NotEnoughFunds {
			available: 0,
			available_disp: amount_to_hr_string(0, false),
			needed: amount_with_fee as u64,
			needed_disp: amount_to_hr_string(amount_with_fee as u64, false),
		})?;
	}

	// The amount with fee is more than the total values of our max outputs
	if total < amount_with_fee && coins.len() == max_outputs {
		return Err(ErrorKind::NotEnoughFunds {
			available: total,
			available_disp: amount_to_hr_string(total, false),
			needed: amount_with_fee as u64,
			needed_disp: amount_to_hr_string(amount_with_fee as u64, false),
		})?;
	}

	let num_outputs = change_outputs + 1;

	// We need to add a change address or amount with fee is more than total
	if total != amount_with_fee {
		fee = tx_fee(coins.len(), num_outputs, 1, None);
		amount_with_fee = amount + fee;

		// Here check if we have enough outputs for the amount including fee otherwise
		// look for other outputs and check again
		while total < amount_with_fee {
			// End the loop if we have selected all the outputs and still not enough funds
			if coins.len() == max_outputs {
				return Err(ErrorKind::NotEnoughFunds {
					available: total as u64,
					available_disp: amount_to_hr_string(total, false),
					needed: amount_with_fee as u64,
					needed_disp: amount_to_hr_string(amount_with_fee as u64, false),
				})?;
			}

			// select some spendable coins from the wallet
			coins = select_coins(
				wallet,
				amount_with_fee,
				current_height,
				minimum_confirmations,
				calculate_max_inputs_in_block(num_outputs),
				selection_strategy_is_use_all,
				parent_key_id,
			)
			.1;
			fee = tx_fee(coins.len(), num_outputs, 1, None);
			total = coins.iter().map(|c| c.value).sum();
			amount_with_fee = amount + fee;
		}
	}
	Ok((coins, total, amount, fee))
}

/// Selects inputs and change for a transaction
pub fn inputs_and_change<T: ?Sized, C, K>(
	coins: &Vec<OutputData>,
	wallet: &mut T,
	amount: u64,
	fee: u64,
	num_change_outputs: usize,
) -> Result<
	(
		Vec<Box<build::Append<K>>>,
		Vec<(u64, Identifier, Option<u64>)>,
	),
	Error,
>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
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
		if coin.is_coinbase {
			parts.push(build::coinbase_input(coin.value, coin.key_id.clone()));
		} else {
			parts.push(build::input(coin.value, coin.key_id.clone()));
		}
	}

	let mut change_amounts_derivations = vec![];

	if change == 0 {
		debug!("No change (sending exactly amount + fee), no change outputs to build");
	} else {
		debug!(
			"Building change outputs: total change: {} ({} outputs)",
			change, num_change_outputs
		);

		let part_change = change / num_change_outputs as u64;
		let remainder_change = change % part_change;

		for x in 0..num_change_outputs {
			// n-1 equal change_outputs and a final one accounting for any remainder
			let change_amount = if x == (num_change_outputs - 1) {
				part_change + remainder_change
			} else {
				part_change
			};

			let change_key = wallet.next_child().unwrap();

			change_amounts_derivations.push((change_amount, change_key.clone(), None));
			parts.push(build::output(change_amount, change_key));
		}
	}

	Ok((parts, change_amounts_derivations))
}

/// Select spendable coins from a wallet.
/// Default strategy is to spend the maximum number of outputs (up to
/// max_outputs). Alternative strategy is to spend smallest outputs first
/// but only as many as necessary. When we introduce additional strategies
/// we should pass something other than a bool in.
/// TODO: Possibly move this into another trait to be owned by a wallet?

pub fn select_coins<T: ?Sized, C, K>(
	wallet: &mut T,
	amount: u64,
	current_height: u64,
	minimum_confirmations: u64,
	max_outputs: usize,
	select_all: bool,
	parent_key_id: &Identifier,
) -> (usize, Vec<OutputData>)
//    max_outputs_available, Outputs
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// first find all eligible outputs based on number of confirmations
	let mut eligible = wallet
		.iter()
		.filter(|out| {
			out.root_key_id == *parent_key_id
				&& out.eligible_to_spend(current_height, minimum_confirmations)
		})
		.collect::<Vec<OutputData>>();

	// max_available can not be bigger than max_outputs
	let max_available = min(eligible.len(), max_outputs);

	// sort eligible outputs by increasing value
	eligible.sort_by_key(|out| out.value);

	// use a sliding window to identify potential sets of possible outputs to spend
	if max_available > 0 {
		for window in eligible.windows(max_available) {
			let windowed_eligibles = window.iter().cloned().collect::<Vec<_>>();
			if let Some(outputs) = select_from(amount, select_all, windowed_eligibles) {
				return (max_available, outputs);
			}
		}
	}

	// we failed to find a suitable set of outputs to spend,
	// so return the largest amount we can so we can provide guidance on what is
	// possible
	eligible.reverse();
	(
		max_available,
		eligible.iter().take(max_available).cloned().collect(),
	)
}

fn select_from(amount: u64, select_all: bool, outputs: Vec<OutputData>) -> Option<Vec<OutputData>> {
	let total = outputs.iter().fold(0, |acc, x| acc + x.value);
	if total >= amount {
		if select_all {
			return Some(outputs.iter().cloned().collect());
		} else {
			let mut selected_amount = 0;
			return Some(
				outputs
					.iter()
					.take_while(|out| {
						let res = selected_amount < amount;
						selected_amount += out.value;
						res
					})
					.cloned()
					.collect(),
			);
		}
	} else {
		None
	}
}
