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
use core::core::amount_to_hr_string;
use libwallet::Error;
use libwallet::types::{OutputData, WalletInfo};
use prettytable;
use std::io::prelude::*;
use term;

/// Display outputs in a pretty way
pub fn outputs(cur_height: u64, validated: bool, outputs: Vec<OutputData>) -> Result<(), Error> {
	let title = format!("Wallet Outputs - Block Height: {}", cur_height);
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
		let num_confirmations = format!("{}", out.num_confirmations(cur_height));
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

	if !validated {
		println!(
			"\nWARNING: Wallet failed to verify data. \
			 The above is from local cache and possibly invalid! \
			 (is your `grin server` offline or broken?)"
		);
	}
	Ok(())
}

/// Display summary info in a pretty way
pub fn info(wallet_info: &WalletInfo) -> Result<(), Error> {
	println!(
		"\n____ Wallet Summary Info at {} ({}) ____\n",
		wallet_info.current_height, wallet_info.data_confirmed_from
	);
	let mut table = table!(
		[bFG->"Total", FG->amount_to_hr_string(wallet_info.total)],
		[bFY->"Awaiting Confirmation", FY->amount_to_hr_string(wallet_info.amount_awaiting_confirmation)],
		[bFY->"Confirmed but Still Locked", FY->amount_to_hr_string(wallet_info.amount_confirmed_but_locked)],
		[bFG->"Currently Spendable", FG->amount_to_hr_string(wallet_info.amount_currently_spendable)],
		[Fw->"---------", Fw->"---------"],
		[Fr->"(Locked by previous transaction)", Fr->amount_to_hr_string(wallet_info.amount_locked)]
	);
	table.set_format(*prettytable::format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
	table.printstd();
	println!();

	if !wallet_info.data_confirmed {
		println!(
			"\nWARNING: Failed to verify wallet contents with grin server. \
			 Above info is maybe not fully updated or invalid! \
			 Check that your `grin server` is OK, or see `wallet help restore`"
		);
	};
	Ok(())
}
