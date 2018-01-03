// Copyright 2016 The Grin Developers
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
use types::*;
use keychain::{Identifier, Keychain};
use util::secp::pedersen;
use util;
use util::LOGGER;

// Transitions a local wallet output from Unconfirmed -> Unspent.
// Also updates the height and lock_height based on latest from the api.
fn refresh_output(out: &mut OutputData, api_out: &api::Output) {
	out.height = api_out.height;
	out.lock_height = api_out.lock_height;

	match out.status {
		OutputStatus::Unconfirmed => {
			out.status = OutputStatus::Unspent;
		}
		_ => (),
	}
}

// Transitions a local wallet output (based on it not being in the node utxo
// set) -
// Unspent -> Spent
// Locked -> Spent
fn mark_spent_output(out: &mut OutputData) {
	match out.status {
		OutputStatus::Unspent | OutputStatus::Locked => out.status = OutputStatus::Spent,
		_ => (),
	}
}

/// Builds a single api query to retrieve the latest output data from the node.
/// So we can refresh the local wallet outputs.
pub fn refresh_outputs(config: &WalletConfig, keychain: &Keychain) -> Result<(), Error> {
	debug!(LOGGER, "Refreshing wallet outputs");
	let mut wallet_outputs: HashMap<pedersen::Commitment, Identifier> = HashMap::new();
	let mut commits: Vec<pedersen::Commitment> = vec![];

	// build a local map of wallet outputs by commits
	// and a list of outputs we want to query the node for
	let _ = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		for out in wallet_data
			.outputs
			.values()
			.filter(|out| out.root_key_id == keychain.root_key_id())
			.filter(|out| out.status != OutputStatus::Spent)
		{
			let commit = keychain.commit_with_key_index(out.value, out.n_child).unwrap();
			commits.push(commit);
			wallet_outputs.insert(commit, out.key_id.clone());
		}
	});

	// build the necessary query params -
 // ?id=xxx&id=yyy&id=zzz
	let query_params: Vec<String> = commits
		.iter()
		.map(|commit| {
			let id = util::to_hex(commit.as_ref().to_vec());
			format!("id={}", id)
		})
		.collect();
	let query_string = query_params.join("&");

	let url = format!(
		"{}/v1/chain/utxos/byids?{}",
		config.check_node_api_http_addr,
		query_string,
	);

	// build a map of api outputs by commit so we can look them up efficiently
	let mut api_outputs: HashMap<pedersen::Commitment, api::Output> = HashMap::new();
	match api::client::get::<Vec<api::Output>>(url.as_str()) {
		Ok(outputs) => for out in outputs {
			api_outputs.insert(out.commit, out);
		},
		Err(e) => {
			// if we got anything other than 200 back from server, don't attempt to refresh the wallet
			// data after
			return Err(Error::Node(e));
		}
	};

	// now for each commit, find the output in the wallet and
 // the corresponding api output (if it exists)
 // and refresh it in-place in the wallet.
 // Note: minimizing the time we spend holding the wallet lock.
	WalletData::with_wallet(&config.data_file_dir, |wallet_data| for commit in commits {
		let id = wallet_outputs.get(&commit).unwrap();
		if let Entry::Occupied(mut output) = wallet_data.outputs.entry(id.to_hex()) {
			match api_outputs.get(&commit) {
				Some(api_output) => refresh_output(&mut output.get_mut(), api_output),
				None => mark_spent_output(&mut output.get_mut()),
			};
		}
	})
}

pub fn get_tip_from_node(config: &WalletConfig) -> Result<api::Tip, Error> {
	let url = format!("{}/v1/chain", config.check_node_api_http_addr);
	api::client::get::<api::Tip>(url.as_str()).map_err(|e| Error::Node(e))
}
