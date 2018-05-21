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
use keychain::{BlindSum, BlindingFactor, Identifier, Keychain};
use libwallet::{aggsig, build};
//TODO: Remove these from here
use types::{build_partial_tx, read_partial_tx, tx_fee, Error, ErrorKind, OutputData, PartialTx,
            PartialTxPhase};

use util::secp::Signature;
use util::secp::key::{PublicKey, SecretKey};
use util::{secp, LOGGER};

use failure::ResultExt;

/// Define the stage that a slate can be in.. for now
/// follows the exchange workflow, but could be made
/// more piecemeal

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SlatePhase {
	/// Sender has initiated
	SenderInitiation,
	/// Receiver has sender's public data, has filled their public data and
	/// part-signed
	ReceiverInitiation,
	/// Sender has all public data, has filled it and signed
	SenderConfirmation,
	/// Reciever has all data, and has signed with their output
	ReceiverConfirmation,
}
/// Public data for each participant in the slate

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ParticipantData {
	/// Id of participant in the transaction. (For now, 0=sender, 1=rec)
	pub id: u64,
	/// Public key corresponding to private blinding factor
	pub public_blind_excess: PublicKey,
	/// Public key corresponding to private nonce
	pub public_nonce: PublicKey,
	/// Public partial signature
	pub part_sig: Option<Signature>,
}

impl ParticipantData {
	/// A helper to return whether this paricipant
	/// has completed round 1 and round 2;
	/// Round 1 has to be completed before instantiation of this struct
	/// anyhow, and for each participant consists of:
	/// -Inputs added to transaction
	/// -Outputs added to transaction
	/// -Public signature nonce chosen and added
	/// -Public contribution to blinding factor chosen and added
	/// Round 2 can only be completed after all participants have
	/// performed round 1, and adds:
	/// -Part sig is filled out
	pub fn is_complete(&self) -> bool {
		self.part_sig.is_some()
	}
}

/// A 'Slate' is passed around to all parties to build up all of the public
/// tranaction data needed to create a finalised tranaction. Callers can pass
/// the slate around by whatever means they choose, (but we can provide some
/// binary or JSON serialisation helpers here).

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Slate {
	/// The number of participants intended to take part in this transaction
	pub num_participants: usize,
	/// Unique transaction ID, selected by sender
	pub id: Uuid,
	/// The core transaction data:
	/// inputs, outputs, kernels, kernel offset
	pub tx: Transaction,
	/// base amount (excluding fee)
	pub amount: u64,
	/// fee amount
	pub fee: u64,
	/// Block height for the transaction
	pub height: u64,
	/// Lock height
	pub lock_height: u64,
	/// Participant data, each participant in the transaction will
	/// insert their public data here. For now, 0 is sender and 1
	/// is receiver, though this will change for multi-party
	pub participant_data: Vec<ParticipantData>,
}

impl Slate {
	/// Create a new slate
	pub fn blank(num_participants: usize) -> Slate {
		Slate {
			num_participants: num_participants,
			id: Uuid::new_v4(),
			tx: Transaction::empty(),
			amount: 0,
			fee: 0,
			height: 0,
			lock_height: 0,
			participant_data: vec![],
		}
	}

	/// Adds selected inputs and outputs to the slate's transaction
	/// Returns blinding factor
	pub fn add_transaction_elements(
		&mut self,
		keychain: &Keychain,
		mut elems: Vec<Box<build::Append>>,
	) -> Result<BlindingFactor, Error> {
		// Append to the exiting transaction
		if self.tx.kernels.len() != 0 {
			elems.insert(0, build::initial_tx(self.tx.clone()));
		}
		let (tx, blind) =
			build::partial_transaction(elems, &keychain).context(ErrorKind::Keychain)?;
		self.tx = tx;
		Ok(blind)
	}

	/// Completes callers part of round 1, adding public key info
	/// to the slate
	pub fn fill_round_1(
		&mut self,
		keychain: &Keychain,
		context_manager: &mut aggsig::ContextManager,
		participant_id: usize,
	) -> Result<(), Error> {
		// Whoever does this first generates the offset
		if self.tx.offset == BlindingFactor::zero() {
			self.generate_offset(keychain, context_manager)?;
		}
		self.add_participant_info(keychain, context_manager, participant_id, None)?;
		Ok(())
	}

	/// Completes caller's part of round 2, completing signatures
	pub fn fill_round_2(
		&mut self,
		keychain: &Keychain,
		context_manager: &mut aggsig::ContextManager,
		participant_id: usize,
	) -> Result<(), Error> {
		self.check_fees()?;
		self.verify_part_sigs(keychain.secp())?;
		let context = context_manager.get_context(&self.id);
		let sig_part = context
			.calculate_partial_sig_with_nonce_sum(
				keychain.secp(),
				&self.pub_nonce_sum(keychain.secp()),
				self.fee,
				self.lock_height,
			)
			.unwrap();
		self.participant_data[participant_id].part_sig = Some(sig_part);
		Ok(())
	}

	/// Creates the final signature, callable by either the sender or recipient
	/// (after phase 3: sender confirmation)
	/// TODO: Only callable by receiver at the moment
	pub fn finalize(
		&mut self,
		keychain: &Keychain,
		output_key_id: &Identifier,
	) -> Result<(), Error> {
		let final_sig = self.finalize_signature(keychain)?;
		self.finalize_transaction(keychain, &final_sig, output_key_id)
	}

	/// Return the sum of public nonces
	fn pub_nonce_sum(&self, secp: &secp::Secp256k1) -> PublicKey {
		let pub_nonces = self.participant_data
			.iter()
			.map(|p| &p.public_nonce)
			.collect();
		PublicKey::from_combination(secp, pub_nonces).unwrap()
	}

	/// Return the sum of public blinding factors
	fn pub_blind_sum(&self, secp: &secp::Secp256k1) -> PublicKey {
		let pub_blinds = self.participant_data
			.iter()
			.map(|p| &p.public_blind_excess)
			.collect();
		PublicKey::from_combination(secp, pub_blinds).unwrap()
	}

	/// Return vector of all partial sigs
	fn part_sigs(&self) -> Vec<&Signature> {
		self.participant_data
			.iter()
			.map(|p| p.part_sig.as_ref().unwrap())
			.collect()
	}

	/// Adds participants public keys to the slate data
	/// and saves participant's transaction context
	/// sec_key can be overriden to replace the blinding
	/// factor (by whoever split the offset)
	fn add_participant_info(
		&mut self,
		keychain: &Keychain,
		context_manager: &aggsig::ContextManager,
		id: usize,
		part_sig: Option<Signature>,
	) -> Result<(), Error> {
		let context = context_manager.get_context(&self.id);

		// Add our public key and nonce to the slate
		let (pub_key, pub_nonce) = context.get_public_keys(keychain.secp());
		self.participant_data.push(ParticipantData {
			id: id as u64,
			public_blind_excess: pub_key,
			public_nonce: pub_nonce,
			part_sig: part_sig,
		});

		Ok(())
	}

	/// Somebody involved needs to generate an offset with their private key
	/// For now, we'll have the transaction initiator be responsible for it
	/// Return offset private key
	fn generate_offset(
		&mut self,
		keychain: &Keychain,
		context_manager: &mut aggsig::ContextManager,
	) -> Result<(), Error> {
		// Generate a random kernel offset here
		// and subtract it from the blind_sum so we create
		// the aggsig context with the "split" key
		let mut context = context_manager.get_context(&self.id);
		self.tx.offset =
			BlindingFactor::from_secret_key(SecretKey::new(&keychain.secp(), &mut thread_rng()));
		let blind_offset = keychain
			.blind_sum(&BlindSum::new()
				.add_blinding_factor(BlindingFactor::from_secret_key(context.sec_key))
				.sub_blinding_factor(self.tx.offset))
			.unwrap();
		context.sec_key = blind_offset
			.secret_key(&keychain.secp())
			.context(ErrorKind::Keychain)?;
		context_manager.save_context(context);
		Ok(())
	}

	/// Checks the fees in the transaction in the given slate are valid
	fn check_fees(&self) -> Result<(), Error> {
		// double check the fee amount included in the partial tx
		// we don't necessarily want to just trust the sender
		// we could just overwrite the fee here (but we won't) due to the sig
		let fee = tx_fee(
			self.tx.inputs.len(),
			self.tx.outputs.len(),
			self.tx.input_proofs_count(),
			None,
		);
		if fee > self.tx.fee() {
			return Err(ErrorKind::FeeDispute {
				sender_fee: self.tx.fee(),
				recipient_fee: fee,
			})?;
		}

		if fee > self.amount + self.fee {
			info!(
				LOGGER,
				"Rejected the transfer because transaction fee ({}) exceeds received amount ({}).",
				amount_to_hr_string(fee),
				amount_to_hr_string(self.amount + self.fee)
			);
			return Err(ErrorKind::FeeExceedsAmount {
				sender_amount: self.amount + self.fee,
				recipient_fee: fee,
			})?;
		}

		Ok(())
	}

	/// Verifies all of the partial signatures in the Slate are valid
	fn verify_part_sigs(&self, secp: &secp::Secp256k1) -> Result<(), Error> {
		// collect public nonces
		for p in self.participant_data.iter() {
			if p.is_complete() {
				if aggsig::verify_partial_sig(
					secp,
					p.part_sig.as_ref().unwrap(),
					&self.pub_nonce_sum(secp),
					&p.public_blind_excess,
					self.fee,
					self.lock_height,
				) == false
				{
					error!(LOGGER, "Partial Sig invalid.");
					return Err(ErrorKind::Signature("Partial Sig invalid."))?;
				}
			}
		}
		Ok(())
	}

	/// This should be callable by either the sender or receiver
	/// once phase 3 is done
	///
	/// Receive Part 3 of interactive transactions from sender, Sender
	/// Confirmation Return Ok/Error
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

	fn finalize_signature(&mut self, keychain: &Keychain) -> Result<Signature, Error> {
		self.verify_part_sigs(keychain.secp())?;

		let part_sigs = self.part_sigs();
		let pub_nonce_sum = self.pub_nonce_sum(keychain.secp());
		let final_pubkey = self.pub_blind_sum(keychain.secp());
		// get the final signature
		let final_sig =
			aggsig::add_signatures(&keychain.secp(), part_sigs, &pub_nonce_sum).unwrap();

		// Calculate the final public key (for our own sanity check)

		// Check our final sig verifies
		let res = aggsig::verify_sig_build_msg(
			&keychain.secp(),
			&final_sig,
			&final_pubkey,
			self.fee,
			self.lock_height,
		);

		if !res {
			error!(LOGGER, "Final aggregated signature invalid.");
			return Err(ErrorKind::Signature("Final aggregated signature invalid."))?;
		}

		Ok(final_sig)
	}

	/// builds a final transaction after the aggregated sig exchange
	fn finalize_transaction(
		&mut self,
		keychain: &Keychain,
		final_sig: &secp::Signature,
		output_key_id: &Identifier,
	) -> Result<(), Error> {
		let root_key_id = keychain.root_key_id();
		let kernel_offset = self.tx.offset;

		self.check_fees()?;

		let mut final_tx = self.tx.clone();

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
		final_tx.kernels[0].excess_sig = final_sig.clone();

		// confirm the kernel verifies successfully before proceeding
		debug!(LOGGER, "Validating final transaction");
		final_tx.kernels[0]
			.verify()
			.context(ErrorKind::Transaction)?;

		// confirm the overall transaction is valid (including the updated kernel)
		let _ = final_tx.validate().context(ErrorKind::Transaction)?;

		debug!(
			LOGGER,
			"Finalized transaction and built output - {:?}, {:?}",
			root_key_id.clone(),
			output_key_id.clone(),
		);

		self.tx = final_tx;
		Ok(())
	}
}

/// EVERYTHING BEYOND HERE IS TO BE REMOVED.
/// Just keeping it here so everything keeps compiling while
/// above API is being refined in tests

pub struct ExchangedTx {
	/// Unique transaction ID, selected by sender
	pub id: Uuid,
	/// The core transaction data, inputs, outputs, etc
	pub tx: Transaction,
	/// Blinding factor for the transaction
	pub blind: BlindingFactor,
	/// Sender's wallet input identifiers ("coins")
	/// selected for the transaction
	/// TODO: Become Identifiers
	pub input_wallet_ids: Vec<OutputData>,
	/// Change output Identifier in sender wallet
	pub change_id: Option<Identifier>,
	/// base amount (excluding fee)
	pub amount: u64,
	/// fee amount
	pub fee: u64,
	/// Block height for the transaction
	pub height: u64,
	/// Lock height
	pub lock_height: u64,
	/// Kernel offset
	pub kernel_offset: Option<BlindingFactor>,
}

/// TODO: Will be removed once above works
/// Initiate a transaction for the aggsig exchange
/// with the given transaction data. tx is updated
/// with the results of the current phase
pub fn sender_initiation(
	keychain: &Keychain,
	context_manager: &mut aggsig::ContextManager,
	tx: &mut ExchangedTx,
) -> Result<(), Error> {
	tx.lock_height = tx.height;

	// Generate a random kernel offset here
	// and subtract it from the blind_sum so we create
	// the aggsig context with the "split" key
	let kernel_offset =
		BlindingFactor::from_secret_key(SecretKey::new(&keychain.secp(), &mut thread_rng()));

	let blind_offset = keychain
		.blind_sum(&BlindSum::new()
			.add_blinding_factor(tx.blind)
			.sub_blinding_factor(kernel_offset))
		.unwrap();

	tx.kernel_offset = Some(kernel_offset);

	// -Sender picks random blinding factors for all outputs it participates in,
	// computes total blinding excess xS -Sender picks random nonce kS
	// -Sender posts inputs, outputs, Message M=fee, xS * G and kS * G to Receiver

	let skey = blind_offset
		.secret_key(&keychain.secp())
		.context(ErrorKind::Keychain)?;

	// Create a new aggsig context
	let mut context = context_manager.create_context(keychain.secp(), &tx.id, skey);
	for input in tx.input_wallet_ids.clone() {
		context.add_output(&input.key_id);
	}

	context_manager.save_context(context);
	Ok(())
}

/// TODO: Remove in favour of above when refactor done
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

/// TODO: Delete after above is integrated
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

///TODO: Remove once above is integrated
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

// TODO: Remove when above is integrated
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
	let sig_vec = vec![other_sig_part, &our_sig_part];
	let final_sig = context
		.calculate_final_sig(&keychain.secp(), sig_vec, &other_pub_nonce)
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

/// TODO: Remove once above version is integrated
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
