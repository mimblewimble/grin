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

use secp;
use checker;
use extkey::ExtendedKey;
use types::{WalletConfig, WalletData};

pub fn show_info(config: &WalletConfig, ext_key: &ExtendedKey) {
	let _ = checker::refresh_outputs(&config, ext_key);
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	// operate within a lock on wallet data
	let _ = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {

		println!("Outputs - ");
		println!("fingerprint, n_child, height, lock_height, status, value");
		println!("----------------------------------");
		for out in &mut wallet_data.outputs {
			let key = ext_key.derive(&secp, out.n_child).unwrap();

			println!(
				"{}, {}, {}, {}, {:?}, {}",
				key.identifier().fingerprint(),
				out.n_child,
				out.height,
				out.lock_height,
				out.status,
				out.value
			);
		}
	});
}
