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

use api;
use types::*;
use keychain::Keychain;
use util;


fn refresh_output(out: &mut OutputData, api_out: Option<api::Output>, tip: &api::Tip) {
	if let Some(api_out) = api_out {
		out.height = api_out.height;
		out.lock_height = api_out.lock_height;

		if out.status == OutputStatus::Locked {
			// leave it Locked locally for now
		} else if api_out.lock_height >= tip.height {
			out.status = OutputStatus::Immature;
		} else {
			out.status = OutputStatus::Unspent;
		}
	} else if vec![OutputStatus::Unspent, OutputStatus::Locked].contains(&out.status) {
		out.status = OutputStatus::Spent;
	}
}

/// Goes through the list of outputs that haven't been spent yet and check
/// with a node whether their status has changed.
pub fn refresh_outputs(
	config: &WalletConfig,
	keychain: &Keychain,
) -> Result<(), Error>{
	let tip = get_tip_from_node(config)?;

	WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
		// check each output that's not spent
		for mut out in wallet_data.outputs
			.values_mut()
			.filter(|out| {
				out.status != OutputStatus::Spent
			})
		{
			// TODO check the pool for unconfirmed
			match get_output_from_node(config, keychain, out.value, out.n_child) {
				Ok(api_out) => refresh_output(&mut out, api_out, &tip),
				Err(_) => {
					// TODO find error with connection and return
					// error!("Error contacting server node at {}. Is it running?",
					// config.check_node_api_http_addr);
				}
			}
		}
	})
}

fn get_tip_from_node(config: &WalletConfig) -> Result<api::Tip, Error> {
	let url = format!("{}/v1/chain/1", config.check_node_api_http_addr);
	api::client::get::<api::Tip>(url.as_str()).map_err(|e| Error::Node(e))
}

// queries a reachable node for a given output, checking whether it's been confirmed
fn get_output_from_node(
	config: &WalletConfig,
	keychain: &Keychain,
	amount: u64,
	derivation: u32,
) -> Result<Option<api::Output>, Error> {
	let pubkey = keychain.derive_pubkey(derivation)?;
	let commit = keychain.commit(amount, &pubkey)?;

	let url = format!(
		"{}/v1/chain/utxo/{}",
		config.check_node_api_http_addr,
		util::to_hex(commit.as_ref().to_vec())
	);
	match api::client::get::<api::Output>(url.as_str()) {
		Ok(out) => Ok(Some(out)),
		Err(api::Error::NotFound) => Ok(None),
		Err(e) => Err(Error::Node(e)),
	}
}
