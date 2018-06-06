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

use api;
use core::consensus::reward;
use core::core::{Output, TxKernel};
use core::global;
use core::ser;
use keychain::Identifier;
use libtx::reward;
use libwallet::error::{Error, ErrorKind};
use libwallet::internal::keys;
use libwallet::types::*;
use util;
use util::LOGGER;
use util::secp::pedersen;

/// Retrieve all of the outputs (doesn't attempt to update from node)
pub fn retrieve_outputs<T: WalletBackend>(
	wallet: &mut T,
	show_spent: bool,
) -> Result<Vec<OutputData>, Error> {
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
pub fn refresh_outputs<T>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend,
{
	let tip = get_tip_from_node(&wallet.node_url())?;
	refresh_output_state(wallet, &tip)?;
	refresh_missing_block_hashes(wallet, &tip)?;
	Ok(())
}

// TODO - this might be slow if we have really old outputs that have never been
// refreshed
fn refresh_missing_block_hashes<T>(wallet: &mut T, tip: &api::Tip) -> Result<(), Error>
where
	T: WalletBackend,
{
	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs_missing_block(wallet)?;

	// nothing to do so return (otherwise we hit the api with a monster query...)
	if wallet_outputs.is_empty() {
		return Ok(());
	}

	debug!(
		LOGGER,
		"Refreshing missing block hashes (and heights) for {} outputs",
		wallet_outputs.len(),
	);

	let id_params: Vec<String> = wallet_outputs
		.keys()
		.map(|commit| format!("id={}", util::to_hex(commit.as_ref().to_vec())))
		.collect();

	let height_params = [format!("start_height={}&end_height={}", 0, tip.height)];

	let mut api_blocks: HashMap<pedersen::Commitment, api::BlockHeaderInfo> = HashMap::new();
	let mut api_merkle_proofs: HashMap<pedersen::Commitment, MerkleProofWrapper> = HashMap::new();

	// Split up into separate requests, to avoid hitting http limits
	for mut query_chunk in id_params.chunks(1000) {
		let url = format!(
			"{}/v1/chain/outputs/byheight?{}",
			wallet.node_url(),
			[&height_params, query_chunk].concat().join("&"),
		);

		match api::client::get::<Vec<api::BlockOutputs>>(url.as_str()) {
			Ok(blocks) => for block in blocks {
				for out in block.outputs {
					api_blocks.insert(out.commit, block.header.clone());
					if let Some(merkle_proof) = out.merkle_proof {
						let wrapper = MerkleProofWrapper(merkle_proof);
						api_merkle_proofs.insert(out.commit, wrapper);
					}
				}
			},
			Err(e) => {
				// if we got anything other than 200 back from server, bye
				return Err(e).context(ErrorKind::Node)?;
			}
		}
	}

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
					output.block = Some(BlockIdentifier::from_hex(&b.hash).unwrap());
					output.height = b.height;
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
pub fn map_wallet_outputs<T>(
	wallet: &mut T,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error>
where
	T: WalletBackend,
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
pub fn map_wallet_outputs_missing_block<T>(
	wallet: &mut T,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error>
where
	T: WalletBackend,
{
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ = wallet.read_wallet(|wallet_data| {
		let keychain = wallet_data.keychain().clone();
		for out in wallet_data.outputs().clone().values().filter(|x| {
			x.root_key_id == wallet_data.keychain().root_key_id() && x.block.is_none()
				&& x.status == OutputStatus::Unspent
		}) {
			let commit = keychain.commit_with_key_index(out.value, out.n_child)?;
			wallet_outputs.insert(commit, out.key_id.clone());
		}
		Ok(())
	});
	Ok(wallet_outputs)
}

/// Apply refreshed API output data to the wallet
pub fn apply_api_outputs<T>(
	wallet: &mut T,
	wallet_outputs: &HashMap<pedersen::Commitment, Identifier>,
	api_outputs: &HashMap<pedersen::Commitment, api::Output>,
) -> Result<(), Error>
where
	T: WalletBackend,
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
fn refresh_output_state<T>(wallet: &mut T, tip: &api::Tip) -> Result<(), Error>
where
	T: WalletBackend,
{
	debug!(LOGGER, "Refreshing wallet outputs");

	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs(wallet)?;

	// build the necessary query params -
	// ?id=xxx&id=yyy&id=zzz
	let query_params: Vec<String> = wallet_outputs
		.keys()
		.map(|commit| format!("id={}", util::to_hex(commit.as_ref().to_vec())))
		.collect();

	// build a map of api outputs by commit so we can look them up efficiently
	let mut api_outputs: HashMap<pedersen::Commitment, api::Output> = HashMap::new();

	for query_chunk in query_params.chunks(1000) {
		let url = format!(
			"{}/v1/chain/outputs/byids?{}",
			wallet.node_url(),
			query_chunk.join("&"),
		);

		match api::client::get::<Vec<api::Output>>(url.as_str()) {
			Ok(outputs) => for out in outputs {
				api_outputs.insert(out.commit.commit(), out);
			},
			Err(e) => {
				// if we got anything other than 200 back from server, don't attempt to refresh
				// the wallet data after
				return Err(e).context(ErrorKind::Node)?;
			}
		}
	}

	apply_api_outputs(wallet, &wallet_outputs, &api_outputs)?;
	clean_old_unconfirmed(wallet, tip)?;
	Ok(())
}

fn clean_old_unconfirmed<T>(wallet: &mut T, tip: &api::Tip) -> Result<(), Error>
where
	T: WalletBackend,
{
	if tip.height < 500 {
		return Ok(());
	}
	wallet.with_wallet(|wallet_data| {
		wallet_data.outputs().retain(|_, ref mut out| {
			!(out.status == OutputStatus::Unconfirmed && out.height > 0
				&& out.height < tip.height - 500)
		});
	})
}

/// Return the chain tip from a given node
pub fn get_tip_from_node(addr: &str) -> Result<api::Tip, Error> {
	let url = format!("{}/v1/chain", addr);
	api::client::get::<api::Tip>(url.as_str())
		.context(ErrorKind::Node)
		.map_err(|e| e.into())
}

/// Retrieve summar info about the wallet
pub fn retrieve_info<T>(wallet: &mut T) -> Result<WalletInfo, Error>
where
	T: WalletBackend,
{
	let result = refresh_outputs(wallet);

	let ret_val = wallet.read_wallet(|wallet_data| {
		let (current_height, from) = match get_tip_from_node(&wallet_data.node_url()) {
			Ok(tip) => (tip.height, "from server node"),
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

		let mut data_confirmed = true;
		if let Err(_) = result {
			data_confirmed = false;
		}
		Ok(WalletInfo {
			current_height: current_height,
			total: unspent_total + unconfirmed_total,
			amount_awaiting_confirmation: unconfirmed_total,
			amount_confirmed_but_locked: unspent_but_locked_total,
			amount_currently_spendable: unspent_total - unspent_but_locked_total,
			amount_locked: locked_total,
			data_confirmed: data_confirmed,
			data_confirmed_from: String::from(from),
		})
	});
	ret_val
}

/// Build a coinbase output and insert into wallet
pub fn build_coinbase<T>(wallet: &mut T, block_fees: &BlockFees) -> Result<CbData, Error>
where
	T: WalletBackend,
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
pub fn receive_coinbase<T>(
	wallet: &mut T,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error>
where
	T: WalletBackend,
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
		&wallet.keychain(),
		&key_id,
		block_fees.fees,
		block_fees.height,
	).unwrap();
	/* .context(ErrorKind::Keychain)?; */
	Ok((out, kern, block_fees))
}
