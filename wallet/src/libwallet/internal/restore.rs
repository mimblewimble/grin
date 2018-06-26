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
use libwallet::Error;
use keychain::{Identifier, Keychain};
use libtx::proof;
use libwallet::types::*;
use util::secp::pedersen;
use util::{self, LOGGER};

// TODO - wrap the many return values in a struct
fn find_outputs_with_key<T, K>(
	wallet: &mut T,
	outputs: Vec<(pedersen::Commitment, pedersen::RangeProof, bool)>,
) -> Result<Vec<(
	pedersen::Commitment,
	Identifier,
	u32,
	u64,
	u64,
	u64,
	bool,
	Option<MerkleProofWrapper>,
)>, Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let mut wallet_outputs: Vec<(
		pedersen::Commitment,
		Identifier,
		u32,
		u64,
		u64,
		u64,
		bool,
		Option<MerkleProofWrapper>,
	)> = Vec::new();

	let max_derivations = 1_000_000;

	info!(LOGGER, "Scanning {} outputs", outputs.len(),);
	let current_chain_height = wallet.get_chain_height()?;

	for output in outputs.iter() {
		let (commit, proof, is_coinbase) = output;
		// attempt to unwind message from the RP and get a value
		// will fail if it's not ours
		let info = proof::rewind(
			wallet.keychain(),
			*commit,
			None,
			*proof,
		)?;

		if !info.success {
			continue;
		}

		// we have a match, now check through our key iterations to find out which one it was
		let mut found = false;
		let mut start_index = 1;

		for i in start_index..max_derivations {
			let key_id = &wallet.keychain().derive_key_id(i as u32)?;
			let b = wallet.keychain().derived_key(key_id)?;
			if info.blinding != b {
				continue;
			}
			found = true;
			// we have a partial match, let's just confirm
			info!(
				LOGGER,
				"Output found: {:?}, key_index: {:?}, amount: {:?}", commit, i, info.value
			);

			let mut merkle_proof = None;
			let commit_str = util::to_hex(commit.as_ref().to_vec());

			if *is_coinbase {
				merkle_proof =
					Some(wallet.create_merkle_proof(&commit_str)?);
			}

			let height = current_chain_height;
			let lock_height = if *is_coinbase {
				height + global::coinbase_maturity()
			} else {
				height
			};

			wallet_outputs.push((
				*commit,
				key_id.clone(),
				i as u32,
				info.value,
				height,
				lock_height,
				*is_coinbase,
				merkle_proof,
			));

			break;
		}
		if !found {
			warn!(
				LOGGER,
				"Very probable matching output found with amount: {} \
				 but didn't match key child key up to {}",
				info.value,
				max_derivations,
			);
		}
	}
	debug!(LOGGER, "Found {} wallet_outputs", wallet_outputs.len(),);

	Ok(wallet_outputs)
}

/// Restore a wallet
pub fn restore<T, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
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
	// this will start here, then lower as outputs are found, moving backwards on
	// the chain
	loop {
		let (highest_index, last_retrieved_index, outputs) = wallet.get_outputs_by_pmmr_index(start_index, batch_size)?;
		info!(
			LOGGER,
			"Retrieved {} outputs, up to index {}. (Highest index: {})",
			outputs.len(),
			highest_index,
			last_retrieved_index,
		);

		let root_key_id = wallet.keychain().root_key_id();
		let result_vec =
			find_outputs_with_key(wallet, outputs.clone())?;
		let mut batch = wallet.batch()?;
		for output in result_vec {
			let _ = batch.save(OutputData {
				root_key_id: root_key_id.clone(),
				key_id: output.1.clone(),
				n_child: output.2,
				value: output.3,
				status: OutputStatus::Unconfirmed,
				height: output.4,
				lock_height: output.5,
				is_coinbase: output.6,
				block: None,
				merkle_proof: output.7,
			});
		}
		batch.commit()?;

		if highest_index == last_retrieved_index {
			break;
		}
		start_index = last_retrieved_index + 1;
	}
	Ok(())
}
