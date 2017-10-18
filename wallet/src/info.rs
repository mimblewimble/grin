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

use checker;
use keychain::Keychain;
use types::{WalletConfig, WalletData};

pub fn show_info(config: &WalletConfig, keychain: &Keychain) {
	let root_key_id = keychain.root_key_id();
	let _ = checker::refresh_outputs(&config, &keychain);

	// operate within a lock on wallet data
	let _ = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		// get the current height via the api
		// if we cannot get the current height use the max height known to the wallet
		let current_height = match checker::get_tip_from_node(config) {
			Ok(tip) => tip.height,
			Err(_) => {
				match wallet_data.outputs.values().map(|out| out.height).max() {
					Some(height) => height,
					None => 0,
				}
			}
		};

		// need to specify a default value here somehow
		let minimum_confirmations = 1;

		println!("Outputs - ");
		println!("key_id, height, lock_height, status, spendable?, coinbase?, value");
		println!("----------------------------------");

		let mut outputs = wallet_data
			.outputs
			.values()
			.filter(|out| out.root_key_id == root_key_id)
			.collect::<Vec<_>>();
		outputs.sort_by_key(|out| out.n_child);
		for out in outputs {
			println!(
				"{}, {}, {}, {:?}, {}, {}, {}",
				out.key_id,
				out.height,
				out.lock_height,
				out.status,
				out.eligible_to_spend(current_height, minimum_confirmations),
				out.is_coinbase,
				out.value,
			);
		}
	});
}
