// Copyright 2017 The Grin Developers
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

//! Provides the JSON/HTTP API for wallets to receive payments. Because
//! receiving money in MimbleWimble requires an interactive exchange, a
//! wallet server that's running at all time is required in many cases.

use bodyparser;
use iron::prelude::*;
use iron::Handler;
use iron::status;
use serde_json;

use api;
use core::consensus::reward;
use core::core::{build, Block, Output, Transaction, TxKernel, amount_to_hr_string};
use core::ser;
use keychain::{Identifier, Keychain};
use types::*;
use util::{LOGGER, to_hex, secp};

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	pub tx_hex: String,
}

/// Receive Part 1 of interactive transactions from sender, Sender Initiation
/// Return result of part 2, Recipient Initation, to sender
/// -Receiver receives inputs, outputs xS * G and kS * G
/// -Receiver picks random blinding factors for all outputs being received, computes total blinding
///     excess xR
/// -Receiver picks random nonce kR
/// -Receiver computes Schnorr challenge e = H(M | kR * G + kS * G)
/// -Receiver computes their part of signature, sR = kR + e * xR
/// -Receiver responds with sR, blinding excess xR * G, public nonce kR * G

fn handle_sender_initiation(
	config: &WalletConfig,
	keychain: &Keychain,
	partial_tx: &PartialTx
) -> Result<PartialTx, Error> {
	let (amount, _sender_pub_blinding, sender_pub_nonce, _sig, tx) = read_partial_tx(keychain, partial_tx)?;

	let root_key_id = keychain.root_key_id();

	// double check the fee amount included in the partial tx
	// we don't necessarily want to just trust the sender
	// we could just overwrite the fee here (but we won't) due to the ecdsa sig
	let fee = tx_fee(tx.inputs.len(), tx.outputs.len() + 1, None);
	if fee != tx.fee {
		return Err(Error::FeeDispute {
			sender_fee: tx.fee,
			recipient_fee: fee,
		});
	}

    if fee > amount {
		info!(
			LOGGER, 
			"Rejected the transfer because transaction fee ({}) exceeds received amount ({}).",
			amount_to_hr_string(fee),
			amount_to_hr_string(amount)
		);
        return Err(Error::FeeExceedsAmount {
            sender_amount: amount,
            recipient_fee: fee,
        });
    }

	let out_amount = amount - fee;

	//First step is just to get the excess sum of the outputs we're participating in
	//Output and key needs to be stored until transaction finalisation time, somehow

	let key_id = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let (key_id, derivation) = next_available_key(&wallet_data, keychain);

		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: out_amount,
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: false,
		});

		key_id
	})?;

	// Still handy for getting the blinding sum
	let (_, blind_sum) = build::transaction(
		vec![
			build::output(out_amount, key_id.clone()),
		],
		keychain,
	)?;

	warn!(LOGGER, "Creating new aggsig context");
	// Create a new aggsig context
	// this will create a new blinding sum and nonce, and store them
	keychain.aggsig_create_context(blind_sum.secret_key());
	keychain.aggsig_add_output(&key_id);

	let sig_part=keychain.aggsig_calculate_partial_sig(&sender_pub_nonce, fee, tx.lock_height).unwrap();

	// Build the response, which should contain sR, blinding excess xR * G, public nonce kR * G
	let mut partial_tx = build_partial_tx(keychain, amount, Some(sig_part), tx);
	partial_tx.phase = PartialTxPhase::ReceiverInitiation;

	Ok(partial_tx)
}

/// Receive Part 3 of interactive transactions from sender, Sender Confirmation
/// Return Ok/Error
/// -Receiver receives sS
/// -Receiver verifies sender's sig, by verifying that kS * G + e *xS * G = sS * G
/// -Receiver calculates final sig as s=(sS+sR, kS * G+kR * G)
/// -Receiver puts into TX kernel:
///
/// Signature S
/// pubkey xR * G+xS * G
/// fee (= M)
/// -Receiver sends completed TX to mempool. responds OK to sender

fn handle_sender_confirmation(
	config: &WalletConfig,
	keychain: &Keychain,
	partial_tx: &PartialTx
) -> Result<PartialTx, Error> {
	let (amount, sender_pub_blinding, sender_pub_nonce, sender_sig_part, tx) = read_partial_tx(keychain, partial_tx)?;
	let sender_sig_part=sender_sig_part.unwrap();
	let res = keychain.aggsig_verify_partial_sig(&sender_sig_part, &sender_pub_nonce, &sender_pub_blinding, tx.fee, tx.lock_height);

	if !res {
		error!(LOGGER, "Partial Sig from sender invalid.");
		return Err(Error::Signature(String::from("Partial Sig from sender invalid.")));
	}

	//Just calculate our sig part again instead of storing
	let our_sig_part=keychain.aggsig_calculate_partial_sig(&sender_pub_nonce, tx.fee, tx.lock_height).unwrap();

	// And the final signature
	let final_sig=keychain.aggsig_calculate_final_sig(&sender_sig_part, &our_sig_part, &sender_pub_nonce).unwrap();

	// Calculate the final public key (for our own sanity check)
	let final_pubkey=keychain.aggsig_calculate_final_pubkey(&sender_pub_blinding).unwrap();

	//Check our final sig verifies
	let res = keychain.aggsig_verify_final_sig_build_msg(&final_sig, &final_pubkey, tx.fee, tx.lock_height);

	if !res {
		error!(LOGGER, "Final aggregated signature invalid.");
		return Err(Error::Signature(String::from("Final aggregated signature invalid.")));
	}


	let final_tx = build_final_transaction(config, keychain, amount, &final_sig, tx.clone())?;
	let tx_hex = to_hex(ser::ser_vec(&final_tx).unwrap());

	let url = format!("{}/v1/pool/push", config.check_node_api_http_addr.as_str());
	api::client::post(url.as_str(), &TxWrapper { tx_hex: tx_hex })
		.map_err(|e| Error::Node(e))?;

	// Return what we've actually posted
	let mut partial_tx = build_partial_tx(keychain, amount, Some(final_sig), tx);
	partial_tx.phase = PartialTxPhase::ReceiverConfirmation;
	Ok(partial_tx)
}

/// Component used to receive coins, implements all the receiving end of the
/// wallet REST API as well as some of the command-line operations.
#[derive(Clone)]
pub struct WalletReceiver {
	pub keychain: Keychain,
	pub config: WalletConfig,
}

impl Handler for WalletReceiver {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let struct_body = req.get::<bodyparser::Struct<PartialTx>>();

		if let Ok(Some(partial_tx)) = struct_body {
			match partial_tx.phase {
				PartialTxPhase::SenderInitiation => {
					let resp_tx=handle_sender_initiation(&self.config, &self.keychain, &partial_tx)
					.map_err(|e| {
						error!(LOGGER, "Phase 1 Sender Initiation -> Problematic partial tx, looks like this: {:?}", partial_tx);
						api::Error::Internal(
							format!("Error processing partial transaction: {:?}", e),
						)})
					.unwrap();
					let json = serde_json::to_string(&resp_tx).unwrap();
					Ok(Response::with((status::Ok, json)))
				},
				PartialTxPhase::SenderConfirmation => {
					let resp_tx=handle_sender_confirmation(&self.config, &self.keychain, &partial_tx)
					.map_err(|e| {
						error!(LOGGER, "Phase 3 Sender Confirmation -> Problematic partial tx, looks like this: {:?}", partial_tx);
						api::Error::Internal(
							format!("Error processing partial transaction: {:?}", e),
						)})
					.unwrap();
					let json = serde_json::to_string(&resp_tx).unwrap();
					Ok(Response::with((status::Ok, json)))
				},
				_=> {
					error!(LOGGER, "Unhandled Phase: {:?}", partial_tx);
					Ok(Response::with((status::BadRequest, "Unhandled Phase")))
				}
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

fn retrieve_existing_key(
	wallet_data: &WalletData,
	key_id: Identifier,
) -> (Identifier, u32) {
	if let Some(existing) = wallet_data.get_output(&key_id) {
		let key_id = existing.key_id.clone();
		let derivation = existing.n_child;
		(key_id, derivation)
	} else {
		panic!("should never happen");
	}
}

fn next_available_key(
	wallet_data: &WalletData,
	keychain: &Keychain,
) -> (Identifier, u32) {
	let root_key_id = keychain.root_key_id();
	let derivation = wallet_data.next_child(root_key_id.clone());
	let key_id = keychain.derive_key_id(derivation).unwrap();
	(key_id, derivation)
}

/// Build a coinbase output and the corresponding kernel
pub fn receive_coinbase(
	config: &WalletConfig,
	keychain: &Keychain,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error> {
	let root_key_id = keychain.root_key_id();

	// Now acquire the wallet lock and write the new output.
	let (key_id, derivation) = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		let key_id = block_fees.key_id();
		let (key_id, derivation) = match key_id {
			Some(key_id) => retrieve_existing_key(&wallet_data, key_id),
			None => next_available_key(&wallet_data, keychain),
		};

		// track the new output and return the stuff needed for reward
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: reward(block_fees.fees),
			status: OutputStatus::Unconfirmed,
			height: 0,
			lock_height: 0,
			is_coinbase: true,
		});

		(key_id, derivation)
	})?;

	debug!(
		LOGGER,
		"Received coinbase and built candidate output - {:?}, {:?}, {}",
		root_key_id.clone(),
		key_id.clone(),
		derivation,
	);

	debug!(LOGGER, "block_fees - {:?}", block_fees);

	let mut block_fees = block_fees.clone();
	block_fees.key_id = Some(key_id.clone());

	debug!(LOGGER, "block_fees updated - {:?}", block_fees);

	let (out, kern) = Block::reward_output(&keychain, &key_id, block_fees.fees)?;
	Ok((out, kern, block_fees))
}

/// builds a final transaction after the aggregated sig exchange
fn build_final_transaction(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	excess_sig: &secp::Signature,
	tx: Transaction,
) -> Result<Transaction, Error> {

	let root_key_id = keychain.root_key_id();

	// double check the fee amount included in the partial tx
 // we don't necessarily want to just trust the sender
 // we could just overwrite the fee here (but we won't) due to the ecdsa sig
	let fee = tx_fee(tx.inputs.len(), tx.outputs.len() + 1, None);
	if fee != tx.fee {
		return Err(Error::FeeDispute {
			sender_fee: tx.fee,
			recipient_fee: fee,
		});
	}

    if fee > amount {
		info!(
			LOGGER, 
			"Rejected the transfer because transaction fee ({}) exceeds received amount ({}).",
			amount_to_hr_string(fee),
			amount_to_hr_string(amount)
		);
        return Err(Error::FeeExceedsAmount {
            sender_amount: amount,
            recipient_fee: fee,
        });
    }

	let out_amount = amount - fee;

	// Get output we created in earlier step
	// TODO: will just be one for now, support multiple later
	let output_vec = keychain.aggsig_get_outputs();

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
		});

		(key_id, derivation)
	})?;

	// Build final transaction, the sum of which should
	// be the same as the exchanged excess values
	let (mut final_tx, _) = build::transaction(
		vec![
			build::initial_tx(tx),
			build::output(out_amount, key_id.clone()),
		],
		keychain,
	)?;

	final_tx.excess_sig = excess_sig.clone();

	// make sure the resulting transaction is valid (could have been lied to on
 // excess).
	let _ = final_tx.validate()?;

	debug!(
		LOGGER,
		"Finalized transaction and built output - {:?}, {:?}, {}",
		root_key_id.clone(),
		key_id.clone(),
		derivation,
	);

	Ok(final_tx)
}
