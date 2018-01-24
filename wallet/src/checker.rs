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

//! Utilities to check the status of all the outputs we have stored in
//! the wallet storage and update them.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use api;
use core::core::hash::Hash;
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

// Transitions a local wallet output (based on it not being in the node utxo
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

// TODO - this might be slow if we have really old outputs that have never been refreshed
fn refresh_missing_block_hashes(config: &WalletConfig, keychain: &Keychain) -> Result<(), Error> {
	// build a local map of wallet outputs keyed by commit
	// and a list of outputs we want to query the node for
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		for out in wallet_data
			.outputs
			.values()
			.filter(|x| {
				x.root_key_id == keychain.root_key_id() &&
				x.block.hash() == Hash::zero() &&
				x.status == OutputStatus::Unspent
			})
		{
			let commit = keychain.commit_with_key_index(out.value, out.n_child).unwrap();
			wallet_outputs.insert(commit, out.key_id.clone());
		}
	});

	// nothing to do so return (otherwise we hit the api with a monster query...)
	if wallet_outputs.is_empty() {
		return Ok(());
	}

	debug!(
		LOGGER,
		"Refreshing missing block hashes (and heights) for {} outputs",
		wallet_outputs.len(),
	);

	let mut id_params: Vec<String> = wallet_outputs
		.keys()
		.map(|commit| {
			let id = util::to_hex(commit.as_ref().to_vec());
			format!("id={}", id)
		})
		.collect();

	let tip = get_tip_from_node(config)?;

	let height_params = format!(
		"start_height={}&end_height={}",
		0,
		tip.height,
	);
	let mut query_params = vec![height_params];
	query_params.append(&mut id_params);

	let url =
		format!(
		"{}/v1/chain/utxos/byheight?{}",
		config.check_node_api_http_addr,
		query_params.join("&"),
	);
	debug!(LOGGER, "{:?}", url);

	let mut api_blocks: HashMap<pedersen::Commitment, api::BlockHeaderInfo> = HashMap::new();
	match api::client::get::<Vec<api::BlockOutputs>>(url.as_str()) {
		Ok(blocks) => {
			for block in blocks {
				for out in block.outputs {
					if let Ok(c) = util::from_hex(String::from(out.commit)) {
						let commit = pedersen::Commitment::from_vec(c);
						api_blocks.insert(commit, block.header.clone());
					}
				}
			}
		}
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(LOGGER, "Refresh failed... unable to contact node: {}", e);
			return Err(Error::Node(e));
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
					output.block = BlockIdentifier::from_str(&b.hash).unwrap();
					output.height = b.height;
				}
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
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let _ = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		for out in wallet_data
			.outputs
			.values()
			.filter(|x| {
				x.root_key_id == keychain.root_key_id() &&
				x.status != OutputStatus::Spent
			})
		{
			let commit = keychain.commit_with_key_index(out.value, out.n_child).unwrap();
			wallet_outputs.insert(commit, out.key_id.clone());
		}
	});

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
	let mut api_utxos: HashMap<pedersen::Commitment, api::Utxo> = HashMap::new();

	let query_string = query_params.join("&");

	let url = format!(
		"{}/v1/chain/utxos/byids?{}",
		config.check_node_api_http_addr, query_string,
	);

	match api::client::get::<Vec<api::Utxo>>(url.as_str()) {
		Ok(outputs) => for out in outputs {
			api_utxos.insert(out.commit, out);
		},
		Err(e) => {
			// if we got anything other than 200 back from server, don't attempt to refresh
			// the wallet data after
			return Err(Error::Node(e));
		}
	};

	// now for each commit, find the output in the wallet and
	// the corresponding api output (if it exists)
	// and refresh it in-place in the wallet.
	// Note: minimizing the time we spend holding the wallet lock.
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| for commit in wallet_outputs.keys() {
		let id = wallet_outputs.get(&commit).unwrap();
		if let Entry::Occupied(mut output) = wallet_data.outputs.entry(id.to_hex()) {
			match api_utxos.get(&commit) {
				Some(_) => mark_unspent_output(&mut output.get_mut()),
				None => mark_spent_output(&mut output.get_mut()),
			};
		}
	})
}

pub fn get_tip_from_node(config: &WalletConfig) -> Result<api::Tip, Error> {
	let url = format!("{}/v1/chain", config.check_node_api_http_addr);
	api::client::get::<api::Tip>(url.as_str()).map_err(|e| Error::Node(e))
}
