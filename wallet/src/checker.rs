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

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use failure::ResultExt;

use api;
use types::*;
use keychain::{Identifier, Keychain};
use util::secp::pedersen;
use util;
use util::LOGGER;

// Transitions a local wallet output from Unconfirmed -> Unspent.
fn mark_unspent_output(out: &mut OutputData) {
	match out.status {
		OutputStatus::Unconfirmed => out.status = OutputStatus::Unspent,
		_ => (),
	}
}

// Transitions a local wallet output (based on it not being in the node output
// set) -
// Unspent -> Spent
// Locked -> Spent
fn mark_spent_output(out: &mut OutputData) {
	match out.status {
		OutputStatus::Unspent => out.status = OutputStatus::Spent,
		OutputStatus::Locked => out.status = OutputStatus::Spent,
		_ => (),
	}
}

pub fn refresh_outputs(config: &WalletConfig, keychain: &Keychain) -> Result<(), Error> {
	refresh_output_state(config, keychain)?;
	refresh_missing_block_hashes(config, keychain)?;
	Ok(())
}

// TODO - this might be slow if we have really old outputs that have never been
// refreshed
fn refresh_missing_block_hashes(config: &WalletConfig, keychain: &Keychain) -> Result<(), Error> {
	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs_missing_block(config, keychain)?;

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
		.map(|commit| {
			let id = util::to_hex(commit.as_ref().to_vec());
			format!("id={}", id)
		})
		.collect();

	let tip = get_tip_from_node(config)?;

	let max_ids_in_query = 1000;
	let mut current_index = 0;

	let mut api_blocks: HashMap<pedersen::Commitment, api::BlockHeaderInfo> = HashMap::new();
	let mut api_merkle_proofs: HashMap<pedersen::Commitment, MerkleProofWrapper> = HashMap::new();

	// Split up into separate requests, to avoid hitting http limits
	loop {
		let q = id_params.clone();
		let mut cur_params: Vec<String> = q.into_iter()
			.skip(current_index)
			.take(max_ids_in_query)
			.collect();

		if cur_params.len() == 0 {
			break;
		}

		debug!(LOGGER, "Splitting query: {} ids", cur_params.len());

		current_index = current_index + cur_params.len();

		let height_params = format!("start_height={}&end_height={}", 0, tip.height,);
		let mut query_params = vec![height_params];
		query_params.append(&mut cur_params);

		let url = format!(
			"{}/v1/chain/outputs/byheight?{}",
			config.check_node_api_http_addr,
			query_params.join("&"),
		);
		debug!(LOGGER, "{:?}", url);

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
				error!(LOGGER, "Refresh failed... unable to contact node: {}", e);
				return Err(e).context(ErrorKind::Node)?;
			}
		}
	}

	// now for each commit, find the output in the wallet and
	// the corresponding api output (if it exists)
	// and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		for commit in wallet_outputs.keys() {
			let id = wallet_outputs.get(&commit).unwrap();
			if let Entry::Occupied(mut output) = wallet_data.outputs.entry(id.to_hex()) {
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
pub fn map_wallet_outputs(
	config: &WalletConfig,
	keychain: &Keychain,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error> {
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ =
		WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
			for out in wallet_data.outputs.values().filter(|x| {
				x.root_key_id == keychain.root_key_id() && x.status != OutputStatus::Spent
			}) {
				let commit = keychain
					.commit_with_key_index(out.value, out.n_child)
					.context(ErrorKind::Keychain)?;
				wallet_outputs.insert(commit, out.key_id.clone());
			}
			Ok(())
		});
	Ok(wallet_outputs)
}

/// As above, but only return unspent outputs with missing block hashes
/// and a list of outputs we want to query the node for
pub fn map_wallet_outputs_missing_block(
	config: &WalletConfig,
	keychain: &Keychain,
) -> Result<HashMap<pedersen::Commitment, Identifier>, Error> {
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		for out in wallet_data.outputs.values().filter(|x| {
			x.root_key_id == keychain.root_key_id() && x.block.is_none()
				&& x.status == OutputStatus::Unspent
		}) {
			let commit = keychain
				.commit_with_key_index(out.value, out.n_child)
				.context(ErrorKind::Keychain)?;
			wallet_outputs.insert(commit, out.key_id.clone());
		}
		Ok(())
	});
	Ok(wallet_outputs)
}

/// Apply refreshed API output data to the wallet
pub fn apply_api_outputs(
	config: &WalletConfig,
	wallet_outputs: &HashMap<pedersen::Commitment, Identifier>,
	api_outputs: &HashMap<pedersen::Commitment, api::Output>,
) -> Result<(), Error> {
	// now for each commit, find the output in the wallet and
	// the corresponding api output (if it exists)
	// and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		for commit in wallet_outputs.keys() {
			let id = wallet_outputs.get(&commit).unwrap();
			if let Entry::Occupied(mut output) = wallet_data.outputs.entry(id.to_hex()) {
				match api_outputs.get(&commit) {
					Some(_) => mark_unspent_output(&mut output.get_mut()),
					None => mark_spent_output(&mut output.get_mut()),
				};
			}
		}
	})
}

/// Builds a single api query to retrieve the latest output data from the node.
/// So we can refresh the local wallet outputs.
fn refresh_output_state(config: &WalletConfig, keychain: &Keychain) -> Result<(), Error> {
	debug!(LOGGER, "Refreshing wallet outputs");

	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let wallet_outputs = map_wallet_outputs(config, keychain)?;
	// build the necessary query params -
	// ?id=xxx&id=yyy&id=zzz
	let query_params: Vec<String> = wallet_outputs
		.keys()
		.map(|commit| {
			let id = util::to_hex(commit.as_ref().to_vec());
			format!("id={}", id)
		})
		.collect();

	// build a map of api outputs by commit so we can look them up efficiently
	let mut api_outputs: HashMap<pedersen::Commitment, api::Output> = HashMap::new();

	let max_ids_in_query = 1000;
	let mut current_index = 0;

	// Split up into separate requests, to avoid hitting http limits
	loop {
		let q = query_params.clone();
		let cur_params: Vec<String> = q.into_iter()
			.skip(current_index)
			.take(max_ids_in_query)
			.collect();

		if cur_params.len() == 0 {
			break;
		}

		debug!(LOGGER, "Splitting query: {} ids", cur_params.len());

		current_index = current_index + cur_params.len();
		let query_string = cur_params.join("&");

		let url = format!(
			"{}/v1/chain/outputs/byids?{}",
			config.check_node_api_http_addr, query_string,
		);

		match api::client::get::<Vec<api::Output>>(url.as_str()) {
			Ok(outputs) => for out in outputs {
				api_outputs.insert(out.commit.commit(), out);
			},
			Err(e) => {
				// if we got anything other than 200 back from server, don't attempt to refresh
				// the wallet data after
				error!(
					LOGGER,
					"Error sending wallet refresh request to server: {:?}", e
				);
				return Err(e).context(ErrorKind::Node)?;
			}
		};
	}
	apply_api_outputs(config, &wallet_outputs, &api_outputs)?;
	Ok(())
}

pub fn get_tip_from_node(config: &WalletConfig) -> Result<api::Tip, Error> {
	let url = format!("{}/v1/chain", config.check_node_api_http_addr);
	api::client::get::<api::Tip>(url.as_str())
		.context(ErrorKind::Node)
		.map_err(|e| e.into())
}
