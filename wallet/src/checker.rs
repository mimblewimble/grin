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
use core::core::{Output, DEFAULT_OUTPUT, COINBASE_OUTPUT};
use core::core::hash::Hashed;
use secp::{self, pedersen};
use util;

use extkey::ExtendedKey;
use types::{WalletConfig, OutputStatus, WalletData};

pub fn refresh_outputs(config: &WalletConfig, ext_key: &ExtendedKey) {
	println!("REFRESH");
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let mut wallet_data = WalletData::read_or_create().expect("Could not open wallet data.");

	let mut changed = 0;
	for out in &mut wallet_data.outputs {
		println!("check out {}", out.n_child);
		if out.status != OutputStatus::Spent {
			let key = ext_key.derive(&secp, out.n_child).unwrap();
			let commitment = secp.commit(out.value, key.key).unwrap();

			// TODO check the pool for unconfirmed
			let out_res = get_output_by_commitment(config, commitment);
			if out_res.is_ok() {
				println!("ok");
				out.status = OutputStatus::Unspent;
				changed += 1;
			}
		}
	}
	if changed > 0 {
		wallet_data.write().unwrap();
	}
}

fn get_output_by_commitment(config: &WalletConfig,
                            commit: pedersen::Commitment)
                            -> Result<Output, api::Error> {
	let url = format!("{}/v1/chain/output/{}",
	                  config.api_http_addr,
	                  util::to_hex(commit.as_ref().to_vec()));
	api::client::get::<Output>(url.as_str())
}
