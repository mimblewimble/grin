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

//! Functions for building partial transactions to be passed
//! around during an interactive wallet exchange
use rand::thread_rng;
use uuid::Uuid;

use checker;
use core::core::{amount_to_hr_string, Committed, Transaction};
use libwallet::{aggsig, build};
use keychain::{BlindSum, BlindingFactor, Identifier, Keychain};
use types::*;
use util::{secp, LOGGER};
use util::secp::key::SecretKey;
use failure::ResultExt;

// TODO: None of these functions should care about the wallet implementation,

/// Initiate a transaction for the aggsig exchange
pub fn sender_initiation(
	config: &WalletConfig,
	keychain: &Keychain,
	tx_id: &Uuid,
	context_manager: &mut aggsig::ContextManager,
	amount: u64,
	minimum_confirmations: u64,
	max_outputs: usize,
	selection_strategy_is_use_all: bool,
) -> Result<PartialTx, Error> {
	checker::refresh_outputs(config, keychain)?;

	let chain_tip = checker::get_tip_from_node(config)?;
	let current_height = chain_tip.height;

	// proof of concept - set lock_height on the tx
	let lock_height = chain_tip.height;

	let (tx, blind, coins, _change_key, amount_with_fee) = build_send_tx(
		config,
		keychain,
		amount,
		current_height,
		minimum_confirmations,
		lock_height,
		max_outputs,
		selection_strategy_is_use_all,
	)?;

	// TODO - wrap this up in build_send_tx or even the build() call?
	// Generate a random kernel offset here
	// and subtract it from the blind_sum so we create
	// the aggsig context with the "split" key
	let kernel_offset =
		BlindingFactor::from_secret_key(SecretKey::new(&keychain.secp(), &mut thread_rng()));

	let blind_offset = keychain
		.blind_sum(&BlindSum::new()
			.add_blinding_factor(blind)
			.sub_blinding_factor(kernel_offset))
		.unwrap();

	//
	// -Sender picks random blinding factors for all outputs it participates in,
	// computes total blinding excess xS -Sender picks random nonce kS
	// -Sender posts inputs, outputs, Message M=fee, xS * G and kS * G to Receiver
	//
	let skey = blind_offset
		.secret_key(&keychain.secp())
		.context(ErrorKind::Keychain)?;

	// Create a new aggsig context
	let mut context = context_manager.create_context(keychain.secp(), &tx_id, skey);
	for coin in coins {
		context.add_output(&coin.key_id);
	}
	let partial_tx = build_partial_tx(
		&context,
		keychain,
		amount_with_fee,
		lock_height,
		kernel_offset,
		None,
		tx,
	);
	context_manager.save_context(context);
	Ok(partial_tx)
}

/// Receive Part 1 of interactive transactions from sender, Sender Initiation
/// Return result of part 2, Recipient Initation, to sender
/// -Receiver receives inputs, outputs xS * G and kS * G
/// -Receiver picks random blinding factors for all outputs being received,
/// computes total blinding
/// excess xR
/// -Receiver picks random nonce kR
/// -Receiver computes Schnorr challenge e = H(M | kR * G + kS * G)
/// -Receiver computes their part of signature, sR = kR + e * xR
/// -Receiver responds with sR, blinding excess xR * G, public nonce kR * G

pub fn recipient_initiation(
	keychain: &Keychain,
	context_manager: &mut aggsig::ContextManager,
	partial_tx: &PartialTx,
	output_key_id: &Identifier,
) -> Result<PartialTx, Error> {
	let (amount, _lock_height, _sender_pub_blinding, sender_pub_nonce, kernel_offset, _sig, tx) =
		read_partial_tx(keychain, partial_tx)?;

	// double check the fee amount included in the partial tx
	// we don't necessarily want to just trust the sender
	// we could just overwrite the fee here (but we won't) due to the sig
	let fee = tx_fee(
		tx.inputs.len(),
		tx.outputs.len() + 1,
		tx.input_proofs_count(),
		None,
	);
	if fee > tx.fee() {
		return Err(ErrorKind::FeeDispute {
			sender_fee: tx.fee(),
			recipient_fee: fee,
		})?;
	}

	if fee > amount {
		info!(
			LOGGER,
			"Rejected the transfer because transaction fee ({}) exceeds received amount ({}).",
			amount_to_hr_string(fee),
			amount_to_hr_string(amount)
		);
		return Err(ErrorKind::FeeExceedsAmount {
			sender_amount: amount,
			recipient_fee: fee,
		})?;
	}

	let out_amount = amount - tx.fee();

	// First step is just to get the excess sum of the outputs we're participating
	// in Output and key needs to be stored until transaction finalisation time,
	// somehow
	// Still handy for getting the blinding sum
	let (_, blind_sum) = build::partial_transaction(
		vec![build::output(out_amount, output_key_id.clone())],
		keychain,
	).context(ErrorKind::Keychain)?;

	// Create a new aggsig context
	// this will create a new blinding sum and nonce, and store them
	let blind = blind_sum
		.secret_key(&keychain.secp())
		.context(ErrorKind::Keychain)?;
	debug!(LOGGER, "Creating new aggsig context");
	let mut context = context_manager.create_context(keychain.secp(), &partial_tx.id, blind);
	context.add_output(output_key_id);
	context.fee = out_amount;

	let sig_part = context
		.calculate_partial_sig(
			keychain.secp(),
			&sender_pub_nonce,
			tx.fee(),
			tx.lock_height(),
		)
		.unwrap();

	// Build the response, which should contain sR, blinding excess xR * G, public
	// nonce kR * G
	let mut partial_tx = build_partial_tx(
		&context,
		keychain,
		amount,
		partial_tx.lock_height,
		kernel_offset,
		Some(sig_part),
		tx,
	);
	partial_tx.phase = PartialTxPhase::ReceiverInitiation;

	context_manager.save_context(context);

	Ok(partial_tx)
}

/// -Sender receives xR * G, kR * G, sR
/// -Sender computes Schnorr challenge e = H(M | kR * G + kS * G)
/// -Sender verifies receivers sig, by verifying that kR * G + e * xR * G =
///  sR * G·
///  -Sender computes their part of signature, sS = kS + e * xS

pub fn sender_confirmation(
	keychain: &Keychain,
	context_manager: &mut aggsig::ContextManager,
	partial_tx: PartialTx,
) -> Result<PartialTx, Error> {
	let context = context_manager.get_context(&partial_tx.id);

	let (amount, lock_height, recp_pub_blinding, recp_pub_nonce, kernel_offset, sig, tx) =
		read_partial_tx(keychain, &partial_tx)?;

	let res = context.verify_partial_sig(
		&keychain.secp(),
		&sig.unwrap(),
		&recp_pub_nonce,
		&recp_pub_blinding,
		tx.fee(),
		lock_height,
	);
	if !res {
		error!(LOGGER, "Partial Sig from recipient invalid.");
		return Err(ErrorKind::Signature("Partial Sig from recipient invalid."))?;
	}

	let sig_part = context
		.calculate_partial_sig(
			&keychain.secp(),
			&recp_pub_nonce,
			tx.fee(),
			tx.lock_height(),
		)
		.unwrap();

	// Build the next stage, containing sS (and our pubkeys again, for the
	// recipient's convenience) offset has not been modified during tx building,
	// so pass it back in
	let mut partial_tx = build_partial_tx(
		&context,
		keychain,
		amount,
		lock_height,
		kernel_offset,
		Some(sig_part),
		tx,
	);
	partial_tx.phase = PartialTxPhase::SenderConfirmation;
	context_manager.save_context(context);
	Ok(partial_tx)
}

/// Receive Part 3 of interactive transactions from sender, Sender Confirmation
/// Return Ok/Error
/// -Receiver receives sS
/// -Receiver verifies sender's sig, by verifying that
/// kS * G + e *xS * G = sS* G
/// -Receiver calculates final sig as s=(sS+sR, kS * G+kR * G)
/// -Receiver puts into TX kernel:
///
/// Signature S
/// pubkey xR * G+xS * G
/// fee (= M)
///
/// Returns completed transaction ready for posting to the chain

pub fn recipient_confirmation(
	config: &WalletConfig,
	keychain: &Keychain,
	context_manager: &mut aggsig::ContextManager,
	partial_tx: &PartialTx,
) -> Result<Transaction, Error> {
	let (
		amount,
		_lock_height,
		sender_pub_blinding,
		sender_pub_nonce,
		kernel_offset,
		sender_sig_part,
		tx,
	) = read_partial_tx(keychain, partial_tx)?;
	let mut context = context_manager.get_context(&partial_tx.id);
	let sender_sig_part = sender_sig_part.unwrap();
	let res = context.verify_partial_sig(
		&keychain.secp(),
		&sender_sig_part,
		&sender_pub_nonce,
		&sender_pub_blinding,
		tx.fee(),
		tx.lock_height(),
	);

	if !res {
		error!(LOGGER, "Partial Sig from sender invalid.");
		return Err(ErrorKind::Signature("Partial Sig from sender invalid."))?;
	}

	// Just calculate our sig part again instead of storing
	let our_sig_part = context
		.calculate_partial_sig(
			&keychain.secp(),
			&sender_pub_nonce,
			tx.fee(),
			tx.lock_height(),
		)
		.unwrap();

	// And the final signature
	let final_sig = context
		.calculate_final_sig(
			&keychain.secp(),
			&sender_sig_part,
			&our_sig_part,
			&sender_pub_nonce,
		)
		.unwrap();

	// Calculate the final public key (for our own sanity check)
	let final_pubkey = context
		.calculate_final_pubkey(&keychain.secp(), &sender_pub_blinding)
		.unwrap();

	// Check our final sig verifies
	let res = context.verify_final_sig_build_msg(
		&keychain.secp(),
		&final_sig,
		&final_pubkey,
		tx.fee(),
		tx.lock_height(),
	);

	if !res {
		error!(LOGGER, "Final aggregated signature invalid.");
		return Err(ErrorKind::Signature("Final aggregated signature invalid."))?;
	}

	build_final_transaction(
		&mut context,
		config,
		keychain,
		amount,
		kernel_offset,
		&final_sig,
		tx.clone(),
	)
}

/// TODO: Probably belongs elsewhere
pub fn next_available_key(wallet_data: &WalletData, keychain: &Keychain) -> (Identifier, u32) {
	let root_key_id = keychain.root_key_id();
	let derivation = wallet_data.next_child(root_key_id.clone());
	let key_id = keychain.derive_key_id(derivation).unwrap();
	(key_id, derivation)
}

/// TODO: Probably belongs elsewhere
pub fn retrieve_existing_key(wallet_data: &WalletData, key_id: Identifier) -> (Identifier, u32) {
	if let Some(existing) = wallet_data.get_output(&key_id) {
		let key_id = existing.key_id.clone();
		let derivation = existing.n_child;
		(key_id, derivation)
	} else {
		panic!("should never happen");
	}
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

/// builds a final transaction after the aggregated sig exchange
fn build_final_transaction(
	context: &mut aggsig::Context,
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	kernel_offset: BlindingFactor,
	excess_sig: &secp::Signature,
	tx: Transaction,
) -> Result<Transaction, Error> {
	let root_key_id = keychain.root_key_id();

	// double check the fee amount included in the partial tx
	// we don't necessarily want to just trust the sender
	// we could just overwrite the fee here (but we won't) due to the ecdsa sig
	let fee = tx_fee(
		tx.inputs.len(),
		tx.outputs.len() + 1,
		tx.input_proofs_count(),
		None,
	);
	if fee > tx.fee() {
		return Err(ErrorKind::FeeDispute {
			sender_fee: tx.fee(),
			recipient_fee: fee,
		})?;
	}

	if fee > amount {
		info!(
			LOGGER,
			"Rejected the transfer because transaction fee ({}) exceeds received amount ({}).",
			amount_to_hr_string(fee),
			amount_to_hr_string(amount)
		);
		return Err(ErrorKind::FeeExceedsAmount {
			sender_amount: amount,
			recipient_fee: fee,
		})?;
	}

	let out_amount = amount - tx.fee();

	// Get output we created in earlier step
	// TODO: will just be one for now, support multiple later
	let output_vec = context.get_outputs();

	// operate within a lock on wallet data
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let (key_id, derivation) = retrieve_existing_key(&wallet_data, output_vec[0].clone());

		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: out_amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
			block: None,
			merkle_proof: None,
		});

		(key_id, derivation)
	})?;

	// Build final transaction, the sum of which should
	// be the same as the exchanged excess values
	let mut final_tx = build::transaction(
		vec![
			build::initial_tx(tx),
			build::output(out_amount, key_id.clone()),
			build::with_offset(kernel_offset),
		],
		keychain,
	).context(ErrorKind::Keychain)?;

	// build the final excess based on final tx and offset
	let final_excess = {
		// TODO - do we need to verify rangeproofs here?
		for x in &final_tx.outputs {
			x.verify_proof().context(ErrorKind::Transaction)?;
		}

		// sum the input/output commitments on the final tx
		let overage = final_tx.fee() as i64;
		let tx_excess = final_tx
			.sum_commitments(overage, None)
			.context(ErrorKind::Transaction)?;

		// subtract the kernel_excess (built from kernel_offset)
		let offset_excess = keychain
			.secp()
			.commit(0, kernel_offset.secret_key(&keychain.secp()).unwrap())
			.unwrap();
		keychain
			.secp()
			.commit_sum(vec![tx_excess], vec![offset_excess])
			.context(ErrorKind::Transaction)?
	};

	// update the tx kernel to reflect the offset excess and sig
	assert_eq!(final_tx.kernels.len(), 1);
	final_tx.kernels[0].excess = final_excess.clone();
	final_tx.kernels[0].excess_sig = excess_sig.clone();

	// confirm the kernel verifies successfully before proceeding
	final_tx.kernels[0]
		.verify()
		.context(ErrorKind::Transaction)?;

	// confirm the overall transaction is valid (including the updated kernel)
	let _ = final_tx.validate().context(ErrorKind::Transaction)?;

	debug!(
		LOGGER,
		"Finalized transaction and built output - {:?}, {:?}, {}",
		root_key_id.clone(),
		key_id.clone(),
		derivation,
	);

	Ok(final_tx)
}
