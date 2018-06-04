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

use core::core;
use libwallet::Error;
use libwallet::types::WalletBackend;
use libwallet::updater;
use prettytable;
use std::io::prelude::*;
use term;

pub fn show_outputs<T: WalletBackend>(wallet: &mut T, show_spent: bool) -> Result<(), Error> {
	let mut local_only = false;
	let res = updater::refresh_outputs(wallet);
	if let Err(_) = res {
		local_only = true;
	};

	let outputs = updater::retrieve_outputs(wallet, show_spent)?;

	let current_height = match updater::get_tip_from_node(wallet.node_url()) {
		Ok(tip) => tip.height,
		Err(_) => {
			local_only = true;
			match outputs.iter().map(|out| out.height).max() {
				Some(height) => height,
				None => 0,
			}
		}
	};

	let title = format!("Wallet Outputs - Block Height: {}", current_height);
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
		let key_id = format!("{}", out.key_id);
		let height = format!("{}", out.height);
		let lock_height = format!("{}", out.lock_height);
		let status = format!("{:?}", out.status);
		let is_coinbase = format!("{}", out.is_coinbase);
		let num_confirmations = format!("{}", out.num_confirmations(current_height));
		let value = format!("{}", core::amount_to_hr_string(out.value));
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

	if local_only {
		println!(
			"\nWARNING: Wallet failed to verify data. \
			 The above is from local cache and possibly invalid! \
			 (is your `grin server` offline or broken?)"
		);
	}
	Ok(())
}
