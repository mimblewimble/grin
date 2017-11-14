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
use core::core;
use types::{WalletConfig, WalletData, OutputStatus};
use prettytable;
use term;
use std::io::prelude::*;

pub fn show_outputs(config: &WalletConfig, keychain: &Keychain, show_spent:bool) {
	let root_key_id = keychain.root_key_id();
	let result = checker::refresh_outputs(&config, &keychain);

	// just read the wallet here, no need for a write lock
	let _ = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		// get the current height via the api
  // if we cannot get the current height use the max height known to the wallet
		let current_height = match checker::get_tip_from_node(config) {
			Ok(tip) => tip.height,
			Err(_) => match wallet_data.outputs.values().map(|out| out.height).max() {
				Some(height) => height,
				None => 0,
			},
		};

		let mut outputs = wallet_data
			.outputs
			.values()
			.filter(|out| out.root_key_id == root_key_id)
			.filter(|out|
				if show_spent {
					true
				} else {
					out.status != OutputStatus::Spent
				})
			.collect::<Vec<_>>();
		outputs.sort_by_key(|out| out.n_child);

		let title=format!("Wallet Outputs - Block Height: {}", current_height);
		println!();
		let mut t = term::stdout().unwrap();
		t.fg(term::color::MAGENTA).unwrap();
		writeln!(t, "{}", title).unwrap();
		t.reset().unwrap();

		let mut table = table!();

		table.set_titles(row![
			bMG->"Key Id",
			bMG->"Block Height",
			bMG->"Locked Until",
			bMG->"Status",
			bMG->"Is Coinbase?",
			bMG->"Num. of Confirmations",
			bMG->"Value"
		]);

		for out in outputs {
			let key_id=format!("{}", out.key_id);
			let height=format!("{}", out.height);
			let lock_height=format!("{}", out.lock_height);
			let status=format!("{:?}", out.status);
			let is_coinbase=format!("{}", out.is_coinbase);
			let num_confirmations=format!("{}", out.num_confirmations(current_height));
			let value=format!("{}", core::amount_to_hr_string(out.value));
			table.add_row(row![
				bFC->key_id,
				bFB->height,
				bFB->lock_height,
				bFR->status,
				bFY->is_coinbase,
				bFB->num_confirmations,
				bFG->value
			]);
		}

		table.set_format(*prettytable::format::consts::FORMAT_NO_COLSEP);
		table.printstd();
		println!();
	});

	if let Err(_) = result {
		println!("WARNING - Showing local data only - Wallet was unable to contact a node to update and verify the outputs shown here.");
	}
}
