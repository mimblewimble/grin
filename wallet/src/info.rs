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

use checker;
use keychain::Keychain;
use types::{WalletConfig, WalletData};

pub fn show_info(
	config: &WalletConfig,
	keychain: &Keychain,
) {
	let fingerprint = keychain.clone().fingerprint();
	let _ = checker::refresh_outputs(&config, &keychain);

	// operate within a lock on wallet data
	let _ = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		println!("Outputs - ");
		println!("identifier, height, lock_height, status, value");
		println!("----------------------------------");

		let mut outputs = wallet_data.outputs
			.values()
			.filter(|out| out.fingerprint == fingerprint)
			.collect::<Vec<_>>();
		outputs.sort_by_key(|out| out.n_child);
		for out in outputs {
			println!(
				"{}..., {}, {}, {:?}, {}",
				out.identifier.fingerprint(),
				out.height,
				out.lock_height,
				out.status,
				out.value
			);
		}
	});
}
