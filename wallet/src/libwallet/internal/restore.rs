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
use keychain::{Identifier, Keychain};
use libtx::proof;
use libwallet::types::*;
use libwallet::Error;
use util::secp::{key::SecretKey, pedersen};
use util::{self, LOGGER};

/// Utility struct for return values from below
struct OutputResult {
	///
	pub commit: pedersen::Commitment,
	///
	pub key_id: Option<Identifier>,
	///
	pub n_child: Option<u32>,
	///
	pub value: u64,
	///
	pub height: u64,
	///
	pub lock_height: u64,
	///
	pub is_coinbase: bool,
	///
	pub merkle_proof: Option<MerkleProofWrapper>,
	///
	pub blinding: SecretKey,
}

fn identify_utxo_outputs<T, K>(
	wallet: &mut T,
	outputs: Vec<(pedersen::Commitment, pedersen::RangeProof, bool)>,
) -> Result<Vec<OutputResult>, Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let mut wallet_outputs: Vec<OutputResult> = Vec::new();

	info!(
		LOGGER,
		"Scanning {} outputs in the current Grin utxo set",
		outputs.len(),
	);
	let current_chain_height = wallet.get_chain_height()?;

	for output in outputs.iter() {
		let (commit, proof, is_coinbase) = output;
		// attempt to unwind message from the RP and get a value
		// will fail if it's not ours
		let info = proof::rewind(wallet.keychain(), *commit, None, *proof)?;

		if !info.success {
			continue;
		}

		info!(
			LOGGER,
			"Output found: {:?}, amount: {:?}", commit, info.value
		);

		let mut merkle_proof = None;
		let commit_str = util::to_hex(commit.as_ref().to_vec());

		if *is_coinbase {
			merkle_proof = Some(wallet.create_merkle_proof(&commit_str)?);
		}

		let height = current_chain_height;
		let lock_height = if *is_coinbase {
			height + global::coinbase_maturity()
		} else {
			height
		};

		wallet_outputs.push(OutputResult {
			commit: *commit,
			key_id: None,
			n_child: None,
			value: info.value,
			height: height,
			lock_height: lock_height,
			is_coinbase: *is_coinbase,
			merkle_proof: merkle_proof,
			blinding: info.blinding,
		});
	}
	Ok(wallet_outputs)
}

/// Attempts to populate a list of outputs with their
/// correct child indices based on the root key
fn populate_child_indices<T, K>(
	wallet: &mut T,
	outputs: &mut Vec<OutputResult>,
	max_derivations: u32,
) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	info!(
		LOGGER,
		"Attempting to populate child indices and key identifiers for {} identified outputs",
		outputs.len()
	);

	// keep track of child keys we've already found, and avoid some EC ops
	let mut found_child_indices: Vec<u32> = vec![];
	for output in outputs.iter_mut() {
		let mut found = false;
		for i in 1..max_derivations {
			// seems to be a bug allowing multiple child keys at the moment
			/*if found_child_indices.contains(&i){
				continue;
			}*/
			let key_id = wallet.keychain().derive_key_id(i as u32)?;
			let b = wallet.keychain().derived_key(&key_id)?;
			if output.blinding != b {
				continue;
			}
			found = true;
			found_child_indices.push(i);
			info!(
				LOGGER,
				"Key index {} found for output {:?}", i, output.commit
			);
			output.key_id = Some(key_id);
			output.n_child = Some(i);
			break;
		}
		if !found {
			warn!(
				LOGGER,
				"Unable to find child key index for: {:?}", output.commit,
			);
		}
	}
	Ok(())
}

/// Restore a wallet
pub fn restore<T, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let max_derivations = 1_000_000;

	// Don't proceed if wallet.dat has anything in it
	let is_empty = wallet.iter().next().is_none();
	if !is_empty {
		error!(
			LOGGER,
			"Not restoring. Please back up and remove existing wallet.dat first."
		);
		return Ok(());
	}

	info!(LOGGER, "Starting restore.");

	let batch_size = 1000;
	let mut start_index = 1;
	let mut result_vec: Vec<OutputResult> = vec![];
	loop {
		let (highest_index, last_retrieved_index, outputs) =
			wallet.get_outputs_by_pmmr_index(start_index, batch_size)?;
		info!(
			LOGGER,
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
		LOGGER,
		"Identified {} wallet_outputs as belonging to this wallet",
		result_vec.len(),
	);

	populate_child_indices(wallet, &mut result_vec, max_derivations)?;

	// Now save what we have
	let root_key_id = wallet.keychain().root_key_id();
	let mut batch = wallet.batch()?;
	for output in result_vec {
		if output.key_id.is_some() && output.n_child.is_some() {
			let _ = batch.save(OutputData {
				root_key_id: root_key_id.clone(),
				key_id: output.key_id.unwrap(),
				n_child: output.n_child.unwrap(),
				value: output.value,
				status: OutputStatus::Unconfirmed,
				height: output.height,
				lock_height: output.lock_height,
				is_coinbase: output.is_coinbase,
				block: None,
				merkle_proof: output.merkle_proof,
			});
		} else {
			warn!(
				LOGGER,
				"Commit {:?} identified but unable to recover key. Output has not been restored.",
				output.commit
			);
		}
	}
	batch.commit()?;
	Ok(())
}
