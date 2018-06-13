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

//! Utilities to check the status of all the outputs we have stored in
//! the wallet storage and update them.

use failure::ResultExt;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use core::consensus::reward;
use core::core::{Output, TxKernel};
use core::global;
use core::ser;
use keychain::{Identifier, Keychain};
use libtx::reward;
use libwallet::error::{Error, ErrorKind};
use libwallet::internal::keys;
use libwallet::types::*;
use util;
use util::LOGGER;
use util::secp::pedersen;

/// Retrieve all of the outputs (doesn't attempt to update from node)
pub fn retrieve_outputs<T, K>(wallet: &mut T, show_spent: bool) -> Result<Vec<OutputData>, Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	let root_key_id = wallet.keychain().clone().root_key_id();

	let mut outputs = vec![];

	// just read the wallet here, no need for a write lock
	let _ = wallet.read_wallet(|wallet_data| {
		outputs = wallet_data
			.outputs()
			.values()
			.filter(|out| out.root_key_id == root_key_id)
			.filter(|out| {
				if show_spent {
					true
				} else {
					out.status != OutputStatus::Spent
				}
			})
			.collect::<Vec<_>>()
			.iter()
			.map(|&o| o.clone())
			.collect();
		outputs.sort_by_key(|out| out.n_child);
		Ok(())
	});
	Ok(outputs)
}

/// Refreshes the outputs in a wallet with the latest information
/// from a node
pub fn refresh_outputs<T, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let height = wallet.get_chain_height(wallet.node_url())?;
	refresh_output_state(wallet, height)?;
	refresh_missing_block_hashes(wallet, height)?;
	Ok(())
}

// TODO - this might be slow if we have really old outputs that have never been
// refreshed
fn refresh_missing_block_hashes<T, K>(wallet: &mut T, height: u64) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs_missing_block(wallet)?;

	let wallet_output_keys = wallet_outputs.keys().map(|commit| commit.clone()).collect();

	// nothing to do so return (otherwise we hit the api with a monster query...)
	if wallet_outputs.is_empty() {
		return Ok(());
	}

	debug!(
		LOGGER,
		"Refreshing missing block hashes (and heights) for {} outputs",
		wallet_outputs.len(),
	);

	let (api_blocks, api_merkle_proofs) =
		wallet.get_missing_block_hashes_from_node(wallet.node_url(), height, wallet_output_keys)?;

	// now for each commit, find the output in the wallet and
	// the corresponding api output (if it exists)
	// and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	wallet.with_wallet(|wallet_data| {
		for commit in wallet_outputs.keys() {
			let id = wallet_outputs.get(&commit).unwrap();
			if let Entry::Occupied(mut output) = wallet_data.outputs().entry(id.to_hex()) {
				if let Some(b) = api_blocks.get(&commit) {
					let output = output.get_mut();
					output.height = b.0;
					output.block = Some(b.1.clone());
					if let Some(merkle_proof) = api_merkle_proofs.get(&commit) {
						output.merkle_proof = Some(merkle_proof.clone());
					}
				}
			}
		}
	})
}

/// build a local map of wallet outputs keyed by commit
/// and a list of outputs we want to query the node for
pub fn map_wallet_outputs<T, K>(
	wallet: &mut T,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ = wallet.read_wallet(|wallet_data| {
		let keychain = wallet_data.keychain().clone();
		let root_key_id = keychain.root_key_id().clone();
		let unspents = wallet_data
			.outputs()
			.values()
			.filter(|x| x.root_key_id == root_key_id && x.status != OutputStatus::Spent);
		for out in unspents {
			let commit = keychain.commit_with_key_index(out.value, out.n_child)?;
			wallet_outputs.insert(commit, out.key_id.clone());
		}
		Ok(())
	});
	Ok(wallet_outputs)
}

/// As above, but only return unspent outputs with missing block hashes
/// and a list of outputs we want to query the node for
pub fn map_wallet_outputs_missing_block<T, K>(
	wallet: &mut T,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ = wallet.read_wallet(|wallet_data| {
		let keychain = wallet_data.keychain().clone();
		let unspents = wallet_data.outputs().values().filter(|x| {
			x.root_key_id == keychain.root_key_id() && x.block.is_none()
				&& x.status == OutputStatus::Unspent
		});
		for out in unspents {
			let commit = keychain.commit_with_key_index(out.value, out.n_child)?;
			wallet_outputs.insert(commit, out.key_id.clone());
		}
		Ok(())
	});
	Ok(wallet_outputs)
}

/// Apply refreshed API output data to the wallet
pub fn apply_api_outputs<T, K>(
	wallet: &mut T,
	wallet_outputs: &HashMap<pedersen::Commitment, Identifier>,
	api_outputs: &HashMap<pedersen::Commitment, String>,
) -> Result<(), Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	// now for each commit, find the output in the wallet and the corresponding
	// api output (if it exists) and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	wallet.with_wallet(|wallet_data| {
		for commit in wallet_outputs.keys() {
			let id = wallet_outputs.get(&commit).unwrap();
			if let Entry::Occupied(mut output) = wallet_data.outputs().entry(id.to_hex()) {
				match api_outputs.get(&commit) {
					Some(_) => output.get_mut().mark_unspent(),
					None => output.get_mut().mark_spent(),
				};
			}
		}
	})
}

/// Builds a single api query to retrieve the latest output data from the node.
/// So we can refresh the local wallet outputs.
fn refresh_output_state<T, K>(wallet: &mut T, height: u64) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	debug!(LOGGER, "Refreshing wallet outputs");

	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs(wallet)?;

	let wallet_output_keys = wallet_outputs.keys().map(|commit| commit.clone()).collect();

	let api_outputs = wallet.get_outputs_from_node(wallet.node_url(), wallet_output_keys)?;
	apply_api_outputs(wallet, &wallet_outputs, &api_outputs)?;
	clean_old_unconfirmed(wallet, height)?;
	Ok(())
}

fn clean_old_unconfirmed<T, K>(wallet: &mut T, height: u64) -> Result<(), Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	if height < 500 {
		return Ok(());
	}
	wallet.with_wallet(|wallet_data| {
		wallet_data.outputs().retain(|_, ref mut out| {
			!(out.status == OutputStatus::Unconfirmed && out.height > 0
				&& out.height < height - 500)
		});
	})
}

/// Retrieve summary info about the wallet
/// caller should refresh first if desired
pub fn retrieve_info<T, K>(wallet: &mut T, refreshed: bool) -> Result<WalletInfo, Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let height_res = wallet.get_chain_height(&wallet.node_url());

	let ret_val = wallet.read_wallet(|wallet_data| {
		let (current_height, from) = match height_res {
			Ok(height) => (height, "from server node"),
			Err(_) => match wallet_data.outputs().values().map(|out| out.height).max() {
				Some(height) => (height, "from wallet"),
				None => (0, "node/wallet unavailable"),
			},
		};
		let mut unspent_total = 0;
		let mut unspent_but_locked_total = 0;
		let mut unconfirmed_total = 0;
		let mut locked_total = 0;
		for out in wallet_data
			.outputs()
			.clone()
			.values()
			.filter(|out| out.root_key_id == wallet_data.keychain().root_key_id())
		{
			if out.status == OutputStatus::Unspent {
				unspent_total += out.value;
				if out.lock_height > current_height {
					unspent_but_locked_total += out.value;
				}
			}
			if out.status == OutputStatus::Unconfirmed && !out.is_coinbase {
				unconfirmed_total += out.value;
			}
			if out.status == OutputStatus::Locked {
				locked_total += out.value;
			}
		}

		Ok(WalletInfo {
			current_height: current_height,
			total: unspent_total + unconfirmed_total,
			amount_awaiting_confirmation: unconfirmed_total,
			amount_confirmed_but_locked: unspent_but_locked_total,
			amount_currently_spendable: unspent_total - unspent_but_locked_total,
			amount_locked: locked_total,
			data_confirmed: refreshed,
			data_confirmed_from: String::from(from),
		})
	});
	ret_val
}

/// Build a coinbase output and insert into wallet
pub fn build_coinbase<T, K>(wallet: &mut T, block_fees: &BlockFees) -> Result<CbData, Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	let (out, kern, block_fees) = receive_coinbase(wallet, block_fees).context(ErrorKind::Node)?;

	let out_bin = ser::ser_vec(&out).context(ErrorKind::Node)?;

	let kern_bin = ser::ser_vec(&kern).context(ErrorKind::Node)?;

	let key_id_bin = match block_fees.key_id {
		Some(key_id) => ser::ser_vec(&key_id).context(ErrorKind::Node)?,
		None => vec![],
	};

	Ok(CbData {
		output: util::to_hex(out_bin),
		kernel: util::to_hex(kern_bin),
		key_id: util::to_hex(key_id_bin),
	})
}

//TODO: Split up the output creation and the wallet insertion
/// Build a coinbase output and the corresponding kernel
pub fn receive_coinbase<T, K>(
	wallet: &mut T,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error>
where
	T: WalletBackend<K>,
	K: Keychain,
{
	let root_key_id = wallet.keychain().root_key_id();

	let height = block_fees.height;
	let lock_height = height + global::coinbase_maturity();

	// Now acquire the wallet lock and write the new output.
	let (key_id, derivation) = wallet.with_wallet(|wallet_data| {
		let key_id = block_fees.key_id();
		let (key_id, derivation) = match key_id {
			Some(key_id) => keys::retrieve_existing_key(wallet_data, key_id),
			None => keys::next_available_key(wallet_data),
		};

		// track the new output and return the stuff needed for reward
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: reward(block_fees.fees),
			status: OutputStatus::Unconfirmed,
			height: height,
			lock_height: lock_height,
			is_coinbase: true,
			block: None,
			merkle_proof: None,
		});

		(key_id, derivation)
	})?;

	debug!(
		LOGGER,
		"receive_coinbase: built candidate output - {:?}, {}",
		key_id.clone(),
		derivation,
	);

	let mut block_fees = block_fees.clone();
	block_fees.key_id = Some(key_id.clone());

	debug!(LOGGER, "receive_coinbase: {:?}", block_fees);

	let (out, kern) = reward::output(
		wallet.keychain(),
		&key_id,
		block_fees.fees,
		block_fees.height,
	).unwrap();
	/* .context(ErrorKind::Keychain)?; */
	Ok((out, kern, block_fees))
}
