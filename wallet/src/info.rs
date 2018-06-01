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

use core::core::amount_to_hr_string;
use libwallet::Error;
use libwallet::types::WalletBackend;
use libwallet::updater;
use prettytable;

pub fn show_info<T>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend,
{
	let wallet_info = updater::retrieve_info(wallet)?;
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
