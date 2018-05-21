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

use core::core::{amount_to_hr_string, Committed, Transaction};
use libwallet::{aggsig, build};
use keychain::{BlindSum, BlindingFactor, Identifier, Keychain};
use types::*; // TODO: Remove this?
use util::{secp, LOGGER};
use util::secp::key::{PublicKey, SecretKey};
use util::secp::Signature;
use failure::ResultExt;

// TODO: None of these functions should care about the wallet implementation,

/// Initiate a transaction for the aggsig exchange
/// with the given transaction data
pub fn sender_initiation(
	keychain: &Keychain,
	tx_id: &Uuid,
	context_manager: &mut aggsig::ContextManager,
	current_height: u64,
	//TODO: Make this nicer, remove wallet-specific OutputData type
	tx_data: (
		Transaction,
		BlindingFactor,
		Vec<OutputData>,
		Option<Identifier>,
		u64,
	),
) -> Result<PartialTx, Error> {
	let lock_height = current_height;

	let (tx, blind, coins, _change_key, amount_with_fee) = tx_data;

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
	context.fee = tx.fee();

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

/// Creates the final signature, callable by either the sender or recipient
/// (after phase 3: sender confirmation)
///
/// TODO: takes a partial Tx that just contains the other party's public
/// info at present, but this should be changed to something more appropriate
pub fn finalize_transaction(
	keychain: &Keychain,
	context_manager: &mut aggsig::ContextManager,
	partial_tx: &PartialTx,
	other_partial_tx: &PartialTx,
	output_key_id: &Identifier,
	output_key_derivation: u32,
) -> Result<Transaction, Error> {
	let (
		_amount,
		_lock_height,
		other_pub_blinding,
		other_pub_nonce,
		kernel_offset,
		other_sig_part,
		tx,
	) = read_partial_tx(keychain, other_partial_tx)?;
	let final_sig = create_final_signature(
		keychain,
		context_manager,
		partial_tx,
		&other_pub_blinding,
		&other_pub_nonce,
		&other_sig_part.unwrap(),
	)?;

	build_final_transaction(
		keychain,
		partial_tx.amount,
		kernel_offset,
		&final_sig,
		tx.clone(),
		output_key_id,
		output_key_derivation,
	)
}

/// This should be callable by either the sender or receiver
/// once phase 3 is done
///
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

fn create_final_signature(
	keychain: &Keychain,
	context_manager: &mut aggsig::ContextManager,
	partial_tx: &PartialTx,
	other_pub_blinding: &PublicKey,
	other_pub_nonce: &PublicKey,
	other_sig_part: &Signature,
) -> Result<Signature, Error> {
	let (_amount, _lock_height, _, _, _kernel_offset, _, tx) =
		read_partial_tx(keychain, partial_tx)?;
	let context = context_manager.get_context(&partial_tx.id);
	let res = context.verify_partial_sig(
		&keychain.secp(),
		&other_sig_part,
		&other_pub_nonce,
		&other_pub_blinding,
		tx.fee(),
		tx.lock_height(),
	);

	if !res {
		error!(LOGGER, "Partial Sig from other party invalid.");
		return Err(ErrorKind::Signature(
			"Partial Sig from other party invalid.",
		))?;
	}

	// Just calculate our sig part again instead of storing
	let our_sig_part = context
		.calculate_partial_sig(
			&keychain.secp(),
			&other_pub_nonce,
			tx.fee(),
			tx.lock_height(),
		)
		.unwrap();

	// And the final signature
	let final_sig = context
		.calculate_final_sig(
			&keychain.secp(),
			&other_sig_part,
			&our_sig_part,
			&other_pub_nonce,
		)
		.unwrap();

	// Calculate the final public key (for our own sanity check)
	let final_pubkey = context
		.calculate_final_pubkey(&keychain.secp(), &other_pub_blinding)
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

	Ok(final_sig)
}

/// builds a final transaction after the aggregated sig exchange
fn build_final_transaction(
	keychain: &Keychain,
	amount: u64,
	kernel_offset: BlindingFactor,
	excess_sig: &secp::Signature,
	tx: Transaction,
	output_key_id: &Identifier,
	output_key_derivation: u32,
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

	// Build final transaction, the sum of which should
	// be the same as the exchanged excess values
	let mut final_tx = build::transaction(
		vec![
			build::initial_tx(tx),
			build::output(out_amount, output_key_id.clone()),
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
	debug!(LOGGER, "Validating final transaction");
	final_tx.kernels[0]
		.verify()
		.context(ErrorKind::Transaction)?;

	// confirm the overall transaction is valid (including the updated kernel)
	let _ = final_tx.validate().context(ErrorKind::Transaction)?;

	debug!(
		LOGGER,
		"Finalized transaction and built output - {:?}, {:?}, {}",
		root_key_id.clone(),
		output_key_id.clone(),
		output_key_derivation,
	);

	Ok(final_tx)
}
