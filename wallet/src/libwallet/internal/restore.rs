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
//! Functions to restore a wallet's outputs from just the master seed

use crate::core::global;
use crate::core::libtx::proof;
use crate::keychain::{ExtKeychain, Identifier, Keychain};
use crate::libwallet::internal::{keys, updater};
use crate::libwallet::types::*;
use crate::libwallet::Error;
use crate::util::secp::{key::SecretKey, pedersen};
use std::collections::HashMap;

/// Utility struct for return values from below
#[derive(Clone)]
struct OutputResult {
	///
	pub commit: pedersen::Commitment,
	///
	pub key_id: Identifier,
	///
	pub n_child: u32,
	///
	pub mmr_index: u64,
	///
	pub value: u64,
	///
	pub height: u64,
	///
	pub lock_height: u64,
	///
	pub is_coinbase: bool,
	///
	pub blinding: SecretKey,
}

fn identify_utxo_outputs<T, C, K>(
	wallet: &mut T,
	outputs: Vec<(pedersen::Commitment, pedersen::RangeProof, bool, u64, u64)>,
) -> Result<Vec<OutputResult>, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut wallet_outputs: Vec<OutputResult> = Vec::new();

	warn!(
		"Scanning {} outputs in the current Grin utxo set",
		outputs.len(),
	);

	for output in outputs.iter() {
		let (commit, proof, is_coinbase, height, mmr_index) = output;
		// attempt to unwind message from the RP and get a value
		// will fail if it's not ours
		let info = proof::rewind(wallet.keychain(), *commit, None, *proof)?;

		if !info.success {
			continue;
		}

		let lock_height = if *is_coinbase {
			*height + global::coinbase_maturity()
		} else {
			*height
		};

		// TODO: Output paths are always going to be length 3 for now, but easy enough to grind
		// through to find the right path if required later
		let key_id = Identifier::from_serialized_path(3u8, &info.message.as_bytes());

		info!(
			"Output found: {:?}, amount: {:?}, key_id: {:?}, mmr_index: {},",
			commit, info.value, key_id, mmr_index,
		);

		wallet_outputs.push(OutputResult {
			commit: *commit,
			key_id: key_id.clone(),
			n_child: key_id.to_path().last_path_index(),
			value: info.value,
			height: *height,
			lock_height: lock_height,
			is_coinbase: *is_coinbase,
			blinding: info.blinding,
			mmr_index: *mmr_index,
		});
	}
	Ok(wallet_outputs)
}

fn collect_chain_outputs<T, C, K>(wallet: &mut T) -> Result<Vec<OutputResult>, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let batch_size = 1000;
	let mut start_index = 1;
	let mut result_vec: Vec<OutputResult> = vec![];
	loop {
		let (highest_index, last_retrieved_index, outputs) = wallet
			.w2n_client()
			.get_outputs_by_pmmr_index(start_index, batch_size)?;
		warn!(
			"Checking {} outputs, up to index {}. (Highest index: {})",
			outputs.len(),
			highest_index,
			last_retrieved_index,
		);

		result_vec.append(&mut identify_utxo_outputs(wallet, outputs.clone())?);

		if highest_index == last_retrieved_index {
			break;
		}
		start_index = last_retrieved_index + 1;
	}
	Ok(result_vec)
}

///
fn restore_missing_output<T, C, K>(
	wallet: &mut T,
	output: OutputResult,
	found_parents: &mut HashMap<Identifier, u32>,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut batch = wallet.batch()?;

	let parent_key_id = output.key_id.parent_path();
	if !found_parents.contains_key(&parent_key_id) {
		found_parents.insert(parent_key_id.clone(), 0);
	}

	let log_id = batch.next_tx_log_id(&parent_key_id)?;
	let entry_type = match output.is_coinbase {
		true => TxLogEntryType::ConfirmedCoinbase,
		false => TxLogEntryType::TxReceived,
	};

	let mut t = TxLogEntry::new(parent_key_id.clone(), entry_type, log_id);
	t.confirmed = true;
	t.amount_credited = output.value;
	t.num_outputs = 1;
	t.update_confirmation_ts();
	batch.save_tx_log_entry(t, &parent_key_id)?;

	let _ = batch.save(OutputData {
		root_key_id: parent_key_id.clone(),
		key_id: output.key_id,
		n_child: output.n_child,
		mmr_index: Some(output.mmr_index),
		value: output.value,
		status: OutputStatus::Unspent,
		height: output.height,
		lock_height: output.lock_height,
		is_coinbase: output.is_coinbase,
		tx_log_entry: Some(log_id),
	});

	let max_child_index = found_parents.get(&parent_key_id).unwrap().clone();
	if output.n_child >= max_child_index {
		found_parents.insert(parent_key_id.clone(), output.n_child);
	}

	batch.commit()?;
	Ok(())
}

///
fn cancel_tx_log_entry<T, C, K>(wallet: &mut T, output: &OutputData) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let parent_key_id = output.key_id.parent_path();
	let updated_tx_entry = if output.tx_log_entry.is_some() {
		let entries = updater::retrieve_txs(
			wallet,
			output.tx_log_entry.clone(),
			None,
			Some(&parent_key_id),
		)?;
		if entries.len() > 0 {
			let mut entry = entries[0].clone();
			match entry.tx_type {
				TxLogEntryType::TxSent => entry.tx_type = TxLogEntryType::TxSentCancelled,
				TxLogEntryType::TxReceived => entry.tx_type = TxLogEntryType::TxReceivedCancelled,
				_ => {}
			}
			Some(entry)
		} else {
			None
		}
	} else {
		None
	};
	let mut batch = wallet.batch()?;
	if let Some(t) = updated_tx_entry {
		batch.save_tx_log_entry(t, &parent_key_id)?;
	}
	batch.commit()?;
	Ok(())
}

/// Check / repair wallet contents
/// assume wallet contents have been freshly updated with contents
/// of latest block
pub fn check_repair<T, C, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// First, get a definitive list of outputs we own from the chain
	warn!("Starting wallet check.");
	let chain_outs = collect_chain_outputs(wallet)?;
	warn!(
		"Identified {} wallet_outputs as belonging to this wallet",
		chain_outs.len(),
	);

	// Now, get all outputs owned by this wallet (regardless of account)
	let wallet_outputs = {
		let res = updater::retrieve_outputs(&mut *wallet, true, None, None)?;
		res
	};

	let mut missing_outs = vec![];
	let mut accidental_spend_outs = vec![];
	let mut locked_outs = vec![];

	// check all definitive outputs exist in the wallet outputs
	for deffo in chain_outs.into_iter() {
		let matched_out = wallet_outputs.iter().find(|wo| wo.1 == deffo.commit);
		match matched_out {
			Some(s) => {
				if s.0.status == OutputStatus::Spent {
					accidental_spend_outs.push((s.0.clone(), deffo.clone()));
				}
				if s.0.status == OutputStatus::Locked {
					locked_outs.push((s.0.clone(), deffo.clone()));
				}
			}
			None => missing_outs.push(deffo),
		}
	}

	// mark problem spent outputs as unspent (confirmed against a short-lived fork, for example)
	for m in accidental_spend_outs.into_iter() {
		let mut o = m.0;
		warn!(
			"Output for {} with ID {} ({:?}) marked as spent but exists in UTXO set. \
			 Marking unspent and cancelling any associated transaction log entries.",
			o.value, o.key_id, m.1.commit,
		);
		o.status = OutputStatus::Unspent;
		// any transactions associated with this should be cancelled
		cancel_tx_log_entry(wallet, &o)?;
		let mut batch = wallet.batch()?;
		batch.save(o)?;
		batch.commit()?;
	}

	let mut found_parents: HashMap<Identifier, u32> = HashMap::new();

	// Restore missing outputs, adding transaction for it back to the log
	for m in missing_outs.into_iter() {
		warn!(
			"Confirmed output for {} with ID {} ({:?}) exists in UTXO set but not in wallet. \
			 Restoring.",
			m.value, m.key_id, m.commit,
		);
		restore_missing_output(wallet, m, &mut found_parents)?;
	}

	// Unlock locked outputs
	for m in locked_outs.into_iter() {
		let mut o = m.0;
		warn!(
			"Confirmed output for {} with ID {} ({:?}) exists in UTXO set and is locked. \
			 Unlocking and cancelling associated transaction log entries.",
			o.value, o.key_id, m.1.commit,
		);
		o.status = OutputStatus::Unspent;
		cancel_tx_log_entry(wallet, &o)?;
		let mut batch = wallet.batch()?;
		batch.save(o)?;
		batch.commit()?;
	}

	let unconfirmed_outs: Vec<&(OutputData, pedersen::Commitment)> = wallet_outputs
		.iter()
		.filter(|o| o.0.status == OutputStatus::Unconfirmed)
		.collect();
	// Delete unconfirmed outputs
	for m in unconfirmed_outs.into_iter() {
		let o = m.0.clone();
		warn!(
			"Unconfirmed output for {} with ID {} ({:?}) not in UTXO set. \
			 Deleting and cancelling associated transaction log entries.",
			o.value, o.key_id, m.1,
		);
		cancel_tx_log_entry(wallet, &o)?;
		let mut batch = wallet.batch()?;
		batch.delete(&o.key_id, &o.mmr_index)?;
		batch.commit()?;
	}

	// restore labels, account paths and child derivation indices
	let label_base = "account";
	let mut index = 1;
	for (path, max_child_index) in found_parents.iter() {
		if *path == ExtKeychain::derive_key_id(2, 0, 0, 0, 0) {
			//default path already exists
			continue;
		}
		let res = wallet.acct_path_iter().find(|e| e.path == *path);
		if let None = res {
			let label = format!("{}_{}", label_base, index);
			keys::set_acct_path(wallet, &label, path)?;
			index = index + 1;
		}
		{
			let mut batch = wallet.batch()?;
			batch.save_child_index(path, max_child_index + 1)?;
		}
	}

	Ok(())
}

/// Restore a wallet
pub fn restore<T, C, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	// Don't proceed if wallet_data has anything in it
	let is_empty = wallet.iter().next().is_none();
	if !is_empty {
		error!("Not restoring. Please back up and remove existing db directory first.");
		return Ok(());
	}

	warn!("Starting restore.");

	let result_vec = collect_chain_outputs(wallet)?;

	warn!(
		"Identified {} wallet_outputs as belonging to this wallet",
		result_vec.len(),
	);

	let mut found_parents: HashMap<Identifier, u32> = HashMap::new();
	// Now save what we have

	for output in result_vec {
		restore_missing_output(wallet, output, &mut found_parents)?;
	}

	// restore labels, account paths and child derivation indices
	let label_base = "account";
	let mut index = 1;
	for (path, max_child_index) in found_parents.iter() {
		if *path == ExtKeychain::derive_key_id(2, 0, 0, 0, 0) {
			//default path already exists
			continue;
		}
		let label = format!("{}_{}", label_base, index);
		keys::set_acct_path(wallet, &label, path)?;
		index = index + 1;
		{
			let mut batch = wallet.batch()?;
			batch.save_child_index(path, max_child_index + 1)?;
		}
	}
	Ok(())
}
