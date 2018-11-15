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

use core::global;
use keychain::{ExtKeychain, Identifier, Keychain};
use libtx::proof;
use libwallet::internal::keys;
use libwallet::types::*;
use libwallet::Error;
use std::collections::HashMap;
use util::secp::{key::SecretKey, pedersen};

/// Utility struct for return values from below
struct OutputResult {
	///
	pub commit: pedersen::Commitment,
	///
	pub key_id: Identifier,
	///
	pub n_child: u32,
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
	outputs: Vec<(pedersen::Commitment, pedersen::RangeProof, bool, u64)>,
) -> Result<Vec<OutputResult>, Error>
where
	T: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	let mut wallet_outputs: Vec<OutputResult> = Vec::new();

	info!(
		"Scanning {} outputs in the current Grin utxo set",
		outputs.len(),
	);

	for output in outputs.iter() {
		let (commit, proof, is_coinbase, height) = output;
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
			"Output found: {:?}, amount: {:?}, parent_key_id: {:?}",
			commit, info.value, key_id
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
		});
	}
	Ok(wallet_outputs)
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

	info!("Starting restore.");

	let batch_size = 1000;
	let mut start_index = 1;
	let mut result_vec: Vec<OutputResult> = vec![];
	loop {
		let (highest_index, last_retrieved_index, outputs) = wallet
			.w2n_client()
			.get_outputs_by_pmmr_index(start_index, batch_size)?;
		info!(
			"Retrieved {} outputs, up to index {}. (Highest index: {})",
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

	info!(
		"Identified {} wallet_outputs as belonging to this wallet",
		result_vec.len(),
	);

	let mut found_parents: HashMap<Identifier, u32> = HashMap::new();
	// Now save what we have
	{
		let mut batch = wallet.batch()?;

		for output in result_vec {
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
			};
		}
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
