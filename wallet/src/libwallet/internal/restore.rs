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

#![allow(unreachable_code)]

use core::global;
use keychain::{Identifier, Keychain};
use libtx::proof;
use libwallet::types::*;
use libwallet::Error;
use util::secp::{key::SecretKey, pedersen};
use util::LOGGER;
use std::time::{Duration, Instant};
use libwallet::internal::updater;

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
	pub blinding: SecretKey,
	///
	pub mmr_index: u64,
}

fn identify_utxo_outputs<T, C, K>(
	wallet: &mut T,
	outputs: Vec<(pedersen::Commitment, pedersen::RangeProof, bool, u64)>,
) -> Result<Vec<OutputResult>, Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	let mut wallet_outputs: Vec<OutputResult> = Vec::new();

	info!(
		LOGGER,
		"Scanning {} outputs in the current Grin UTXO set",
		outputs.len(),
	);

	for output in outputs.into_iter() {
		let (commit, proof, is_coinbase, mmr_index) = output;
		// attempt to unwind message from the RP and get a value
		// will fail if it's not ours
		let info = proof::rewind(wallet.keychain(), commit, None, proof)?;

		if !info.success {
			continue;
		}

		info!(
			LOGGER,
			"Output found: {:?}, amount: {:?}, coinbase: {:?}, mmr_index: {}", commit, info.value, is_coinbase, mmr_index
		);

		wallet_outputs.push(OutputResult {
			commit: commit,
			key_id: None,
			n_child: None,
			value: info.value,
			height: 0,
			lock_height: 0,
			is_coinbase: is_coinbase,
			blinding: info.blinding,
			mmr_index: mmr_index,
		});
	}
	Ok(wallet_outputs)
}

/// Attempts to populate a list of outputs with their
/// correct child indices based on the root key
fn populate_child_indices<T, C, K>(
	wallet: &mut T,
	outputs: &mut Vec<OutputResult>,
	max_derivations: u32,
) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
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
pub fn restore<T, C, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
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
		let (highest_index, last_retrieved_index, outputs) = wallet
			.client()
			.get_outputs_by_pmmr_index(start_index, batch_size)?;

		info!(
			LOGGER,
			"Retrieved {} outputs, up to index {}. (Highest index: {})",
			outputs.len(),
			last_retrieved_index,
			highest_index,
		);

		let mut my_utxo_outputs = identify_utxo_outputs(wallet, outputs)?;
		result_vec.append(&mut my_utxo_outputs);

		if highest_index == last_retrieved_index {
			break;
		}
		start_index = last_retrieved_index + 1;
	}

	info!(
		LOGGER,
		"Identified {} wallet_outputs as belonging to this wallet. Resolving block heights.",
		result_vec.len(),
	);

	{
		let batch_size = 1000;
		let mut start_height = 1;
		let mut cur_block_mmr = (0, 0); // (height, mmr_size)
		let mut result_iter = result_vec.iter_mut().peekable();

		'outer: loop {
			info!(
				LOGGER,
				"Fetching headers... start_height({})",
				start_height,
			);

			let (tip_height, last_retrieved_height, headers) = wallet
				.client()
				.get_block_output_mmr_size(start_height, batch_size)?;
			info!(
				LOGGER,
				"Retrieved {} headers for mmr_size, up to height {}. (Tip height: {})",
				headers.len(),
				last_retrieved_height,
				tip_height,
			);
			start_height = last_retrieved_height + 1;

			let mut header_iter = headers.iter();
			'inner: loop {
				if let Some(output) = result_iter.peek() {
					// mmr_index starts with 1
					while output.mmr_index > cur_block_mmr.1 {
						if let Some(h) = header_iter.next() {
							cur_block_mmr = *h;
							debug!(
								LOGGER,
								"HEADER height({}) mmr_size({}) mmr_index({})", cur_block_mmr.0, cur_block_mmr.1, output.mmr_index
							);
						} else {
							// no more header
							debug!(
								LOGGER,
								"no more header continue to 'outer loop..."
							);
							continue 'outer;
						}
					}
				} else {
					// no more result
					debug!(
						LOGGER,
						"breaking 'outer"	,
					);
					break 'outer;
				}

				let out = result_iter.next().unwrap();
				out.height = cur_block_mmr.0;

				out.lock_height = if out.is_coinbase {
					out.height + global::coinbase_maturity()
				} else {
					out.height
				};

				info!(
					LOGGER,
					"Found height({}), {:?}, mmr({}|{}), cb({})",
					out.height,
					out.commit,
					out.mmr_index,
					cur_block_mmr.1,
					out.is_coinbase,
				);
			}
			panic!("should not reach here!");
		}
	}

	populate_child_indices(wallet, &mut result_vec, max_derivations)?;

	// Now save what we have
	{
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
					tx_log_entry: None,
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
	}
	info!(
		LOGGER,
		"Refreshing outputs"
	);
	updater::refresh_outputs(wallet)?;
	Ok(())
}
