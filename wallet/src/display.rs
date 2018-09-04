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

use core::core::{self, amount_to_hr_string};
use libwallet::types::{OutputData, TxLogEntry, WalletInfo};
use libwallet::Error;
use prettytable;
use std::io::prelude::Write;
use term;
use util;
use util::secp::pedersen;

/// Display outputs in a pretty way
pub fn outputs(
	cur_height: u64,
	validated: bool,
	outputs: Vec<(OutputData, pedersen::Commitment)>,
) -> Result<(), Error> {
	let title = format!("Wallet Outputs - Block Height: {}", cur_height);
	println!();
	let mut t = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	t.reset().unwrap();

	let mut table = table!();

	table.set_titles(row![
		bMG->"Output Commitment",
		bMG->"Block Height",
		bMG->"Locked Until",
		bMG->"Status",
		bMG->"Coinbase?",
		bMG->"# Confirms",
		bMG->"Value",
		bMG->"Tx"
	]);

	for (out, commit) in outputs {
		let commit = format!("{}", util::to_hex(commit.as_ref().to_vec()));
		let height = format!("{}", out.height);
		let lock_height = format!("{}", out.lock_height);
		let status = format!("{:?}", out.status);
		let is_coinbase = format!("{}", out.is_coinbase);
		let num_confirmations = format!("{}", out.num_confirmations(cur_height));
		let value = format!("{}", core::amount_to_hr_string(out.value, false));
		let tx = match out.tx_log_entry {
			None => "".to_owned(),
			Some(t) => t.to_string(),
		};
		table.add_row(row![
			bFC->commit,
			bFB->height,
			bFB->lock_height,
			bFR->status,
			bFY->is_coinbase,
			bFB->num_confirmations,
			bFG->value,
			bFC->tx,
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

/// Display transaction log in a pretty way
pub fn txs(
	cur_height: u64,
	validated: bool,
	txs: Vec<TxLogEntry>,
	include_status: bool,
) -> Result<(), Error> {
	let title = format!("Transaction Log - Block Height: {}", cur_height);
	println!();
	let mut t = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	t.reset().unwrap();

	let mut table = table!();

	table.set_titles(row![
		bMG->"Id",
		bMG->"Type",
		bMG->"Shared Transaction Id",
		bMG->"Creation Time",
		bMG->"Confirmed?",
		bMG->"Confirmation Time",
		bMG->"Num. Inputs",
		bMG->"Num. Outputs",
		bMG->"Amount Credited",
		bMG->"Amount Debited",
		bMG->"Fee",
		bMG->"Net Difference",
		bMG->"Tx Data",
	]);

	for t in txs {
		let id = format!("{}", t.id);
		let slate_id = match t.tx_slate_id {
			Some(m) => format!("{}", m),
			None => "None".to_owned(),
		};
		let entry_type = format!("{}", t.tx_type);
		let creation_ts = format!("{}", t.creation_ts.format("%Y-%m-%d %H:%M:%S"));
		let confirmation_ts = match t.confirmation_ts {
			Some(m) => format!("{}", m.format("%Y-%m-%d %H:%M:%S")),
			None => "None".to_owned(),
		};
		let confirmed = format!("{}", t.confirmed);
		let num_inputs = format!("{}", t.num_inputs);
		let num_outputs = format!("{}", t.num_outputs);
		let amount_debited_str = core::amount_to_hr_string(t.amount_debited, true);
		let amount_credited_str = core::amount_to_hr_string(t.amount_credited, true);
		let fee = match t.fee {
			Some(f) => format!("{}", core::amount_to_hr_string(f, true)),
			None => "None".to_owned(),
		};
		let net_diff = if t.amount_credited >= t.amount_debited {
			core::amount_to_hr_string(t.amount_credited - t.amount_debited, true)
		} else {
			format!(
				"-{}",
				core::amount_to_hr_string(t.amount_debited - t.amount_credited, true)
			)
		};
		let tx_data = match t.tx_hex {
			Some(_) => format!("Exists"),
			None => "None".to_owned(),
		};
		table.add_row(row![
			bFC->id,
			bFC->entry_type,
			bFC->slate_id,
			bFB->creation_ts,
			bFC->confirmed,
			bFB->confirmation_ts,
			bFC->num_inputs,
			bFC->num_outputs,
			bFG->amount_credited_str,
			bFR->amount_debited_str,
			bFR->fee,
			bFY->net_diff,
			bFb->tx_data,
		]);
	}

	table.set_format(*prettytable::format::consts::FORMAT_NO_COLSEP);
	table.printstd();
	println!();

	if !validated && include_status {
		println!(
			"\nWARNING: Wallet failed to verify data. \
			 The above is from local cache and possibly invalid! \
			 (is your `grin server` offline or broken?)"
		);
	}
	Ok(())
}
/// Display summary info in a pretty way
pub fn info(wallet_info: &WalletInfo, validated: bool) {
	println!(
		"\n____ Wallet Summary Info as of {} ____\n",
		wallet_info.last_confirmed_height
	);
	let mut table = table!(
		[bFG->"Total", FG->amount_to_hr_string(wallet_info.total, false)],
		[bFY->"Awaiting Confirmation", FY->amount_to_hr_string(wallet_info.amount_awaiting_confirmation, false)],
		[bFY->"Immature Coinbase", FY->amount_to_hr_string(wallet_info.amount_immature, false)],
		[bFG->"Currently Spendable", FG->amount_to_hr_string(wallet_info.amount_currently_spendable, false)],
		[Fw->"---------", Fw->"---------"],
		[Fr->"(Locked by previous transaction)", Fr->amount_to_hr_string(wallet_info.amount_locked, false)]
	);
	table.set_format(*prettytable::format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
	table.printstd();
	println!();
	if !validated {
		println!(
			"\nWARNING: Wallet failed to verify data against a live chain. \
			 The above is from local cache and only valid up to the given height! \
			 (is your `grin server` offline or broken?)"
		);
	}
}
