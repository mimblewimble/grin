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
use core::core::amount_to_hr_string;
use types::{WalletConfig, WalletData, OutputStatus, StatsData, format_transfer_timestamp};
use prettytable;
use term;
use std::io::prelude::*;
use std::cmp::Reverse;

pub fn show_info(config: &WalletConfig, keychain: &Keychain) {
	let result = checker::refresh_outputs(&config, &keychain);


	let _ = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		let current_height = match checker::get_tip_from_node(config) {
			Ok(tip) => tip.height,
			Err(_) => match wallet_data.outputs.values().map(|out| out.height).max() {
				Some(height) => height,
				None => 0,
			},
		};
		let mut unspent_total=0;
		let mut unspent_but_locked_total=0;
		let mut unconfirmed_total=0;
		let mut locked_total=0;
		for out in wallet_data
			.outputs
			.values()
			.filter(|out| out.root_key_id == keychain.root_key_id())
		{
			if out.status == OutputStatus::Unspent {
				unspent_total+=out.value;
				if out.lock_height > current_height {
						unspent_but_locked_total+=out.value;
				}
			}
			if out.status == OutputStatus::Unconfirmed && !out.is_coinbase {
				unconfirmed_total+=out.value;
			}
			if out.status == OutputStatus::Locked {
				locked_total+=out.value;
			}
		};


		println!();
		let title=format!("Wallet Summary Info - Block Height: {}", current_height);
		let mut t = term::stdout().unwrap();
		t.fg(term::color::MAGENTA).unwrap();
		writeln!(t, "--------------------------").unwrap(); // separator above, to avoid bug font
		writeln!(t, "{}\n", title).unwrap(); // when pasted in gitter.
		t.reset().unwrap();

		let mut table = table!(
			[bFG->"Total", FG->amount_to_hr_string(unspent_total+unconfirmed_total)],
			[bFY->"Awaiting Confirmation", FY->amount_to_hr_string(unconfirmed_total)],
			[bFY->"Confirmed but Still Locked", FY->amount_to_hr_string(unspent_but_locked_total)],
			[bFG->"Currently Spendable", FG->amount_to_hr_string(unspent_total-unspent_but_locked_total)],
			[Fw->"---------", Fw->"---------"],
			[Fr->"(Locked by previous transaction)", Fr->amount_to_hr_string(locked_total)]
		);
		table.set_format(*prettytable::format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
		table.printstd();
		println!();
	});

	if let Err(_) = result {
		println!(
		"\nWARNING: Wallet failed to verify data. \
		 The above is from local cache and possibly invalid! \
		 (is your `grin server` offline or broken?)"
		);
	} else {
		show_stats(config);
	}
}

/// Display the history of transfers of sending and receiving.
/// Each row shows transfer type (Sent, Received), amount, Sent or Received at and receiving wallet address.
fn show_stats(config: &WalletConfig) {
	let _ = StatsData::read_stats(&config.data_file_dir, |stats_data| {
		let total_transfers = stats_data.transfers.len();
		let title=format!("Grin Coin Transfer List - Total Transfers: {}", total_transfers);

		println!();
		let mut t = term::stdout().unwrap();
		t.fg(term::color::MAGENTA).unwrap();
		writeln!(t, "{}", title).unwrap();
		t.reset().unwrap();
		println!("Please note that 'sent or received at' indicates the date & time\n your wallet sent or received grin coins at.");

		let mut stats = stats_data
			.transfers
			.values()
			.collect::<Vec<_>>();
		stats.sort_by_key(|stat| Reverse(stat.sent_or_received_at));

		let mut table = table!();
		// Set table titles.
		table.add_row(row![
			bFB->"Transfer",
			bFB->"Amount",
			bFB->"Sent or Received at",
			bFB->"Receiving Wallet Address"
		]);

		// Add a row per time
		for txr in stats {
			let tx_type = format!("{:?}", txr.tx_type);
			let amount = format!("{}", amount_to_hr_string(txr.amount));
			let sent_or_received_at = format_transfer_timestamp(txr.sent_or_received_at);
			table.add_row(row![
				bFG->tx_type,
				bFM->amount,
				bFd->sent_or_received_at,
				bFd->txr.receiving_wallet_address
			]);
		};

		table.printstd();
		println!();
	});

}
