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

use checker;
use core::core::amount_to_hr_string;
use libwallet::types::*;
use prettytable;

pub fn show_info<T>(wallet: &mut T)
where
	T: WalletBackend,
{
	let wallet_info = retrieve_info(wallet);
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
	}
}

pub fn retrieve_info<T>(wallet: &mut T) -> WalletInfo
where
	T: WalletBackend,
{
	let result = checker::refresh_outputs(wallet);

	let ret_val = wallet.read_wallet(|wallet_data| {
		let (current_height, from) = match checker::get_tip_from_node(&wallet_data.node_url()) {
			Ok(tip) => (tip.height, "from server node"),
			Err(_) => match wallet_data.outputs().values().map(|out| out.height).max() {
				Some(height) => (height, "from wallet"),
				None => (0, "node/wallet unavailable"),
			},
		};
		let mut unspent_total = 0;
		let mut unspent_but_locked_total = 0;
		let mut unconfirmed_total = 0;
		let mut locked_total = 0;
		for out in wallet_data
			.outputs()
			.clone()
			.values()
			.filter(|out| out.root_key_id == wallet_data.keychain().root_key_id())
		{
			if out.status == OutputStatus::Unspent {
				unspent_total += out.value;
				if out.lock_height > current_height {
					unspent_but_locked_total += out.value;
				}
			}
			if out.status == OutputStatus::Unconfirmed && !out.is_coinbase {
				unconfirmed_total += out.value;
			}
			if out.status == OutputStatus::Locked {
				locked_total += out.value;
			}
		}

		let mut data_confirmed = true;
		if let Err(_) = result {
			data_confirmed = false;
		}
		Ok(WalletInfo {
			current_height: current_height,
			total: unspent_total + unconfirmed_total,
			amount_awaiting_confirmation: unconfirmed_total,
			amount_confirmed_but_locked: unspent_but_locked_total,
			amount_currently_spendable: unspent_total - unspent_but_locked_total,
			amount_locked: locked_total,
			data_confirmed: data_confirmed,
			data_confirmed_from: String::from(from),
		})
	});
	ret_val.unwrap()
}
