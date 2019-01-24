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

use crate::core::core::{self, amount_to_hr_string};
use crate::core::global;
use crate::libwallet::types::{AcctPathMapping, OutputData, OutputStatus, TxLogEntry, WalletInfo};
use crate::libwallet::Error;
use crate::util;
use crate::util::secp::pedersen;
use prettytable;
use std::io::prelude::Write;
use term;

/// Display outputs in a pretty way
pub fn outputs(
	account: &str,
	cur_height: u64,
	validated: bool,
	outputs: Vec<(OutputData, pedersen::Commitment)>,
	dark_background_color_scheme: bool,
) -> Result<(), Error> {
	let title = format!(
		"Wallet Outputs - Account '{}' - Block Height: {}",
		account, cur_height
	);
	println!();
	let mut t = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	t.reset().unwrap();

	let mut table = table!();

	table.set_titles(row![
		bMG->"Output Commitment",
		bMG->"MMR Index",
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
		let index = match out.mmr_index {
			None => "None".to_owned(),
			Some(t) => t.to_string(),
		};
		let height = format!("{}", out.height);
		let lock_height = format!("{}", out.lock_height);
		let is_coinbase = format!("{}", out.is_coinbase);

		// Mark unconfirmed coinbase outputs as "Mining" instead of "Unconfirmed"
		let status = match out.status {
			OutputStatus::Unconfirmed if out.is_coinbase => "Mining".to_string(),
			_ => format!("{}", out.status),
		};

		let num_confirmations = format!("{}", out.num_confirmations(cur_height));
		let value = format!("{}", core::amount_to_hr_string(out.value, false));
		let tx = match out.tx_log_entry {
			None => "".to_owned(),
			Some(t) => t.to_string(),
		};

		if dark_background_color_scheme {
			table.add_row(row![
				bFC->commit,
				bFB->index,
				bFB->height,
				bFB->lock_height,
				bFR->status,
				bFY->is_coinbase,
				bFB->num_confirmations,
				bFG->value,
				bFC->tx,
			]);
		} else {
			table.add_row(row![
				bFD->commit,
				bFB->index,
				bFB->height,
				bFB->lock_height,
				bFR->status,
				bFD->is_coinbase,
				bFB->num_confirmations,
				bFG->value,
				bFD->tx,
			]);
		}
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
	account: &str,
	cur_height: u64,
	validated: bool,
	txs: &Vec<TxLogEntry>,
	include_status: bool,
	dark_background_color_scheme: bool,
) -> Result<(), Error> {
	let title = format!(
		"Transaction Log - Account '{}' - Block Height: {}",
		account, cur_height
	);
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
		bMG->"Num. \nInputs",
		bMG->"Num. \nOutputs",
		bMG->"Amount \nCredited",
		bMG->"Amount \nDebited",
		bMG->"Fee",
		bMG->"Net \nDifference",
		bMG->"Tx \nData",
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
		let tx_data = match t.stored_tx {
			Some(_) => "Yes".to_owned(),
			None => "None".to_owned(),
		};
		if dark_background_color_scheme {
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
		} else {
			if t.confirmed {
				table.add_row(row![
					bFD->id,
					bFb->entry_type,
					bFD->slate_id,
					bFB->creation_ts,
					bFg->confirmed,
					bFB->confirmation_ts,
					bFD->num_inputs,
					bFD->num_outputs,
					bFG->amount_credited_str,
					bFD->amount_debited_str,
					bFD->fee,
					bFG->net_diff,
					bFB->tx_data,
				]);
			} else {
				table.add_row(row![
					bFD->id,
					bFb->entry_type,
					bFD->slate_id,
					bFB->creation_ts,
					bFR->confirmed,
					bFB->confirmation_ts,
					bFD->num_inputs,
					bFD->num_outputs,
					bFG->amount_credited_str,
					bFD->amount_debited_str,
					bFD->fee,
					bFG->net_diff,
					bFB->tx_data,
				]);
			}
		}
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
pub fn info(
	account: &str,
	wallet_info: &WalletInfo,
	validated: bool,
	dark_background_color_scheme: bool,
) {
	println!(
		"\n____ Wallet Summary Info - Account '{}' as of height {} ____\n",
		account, wallet_info.last_confirmed_height,
	);

	let mut table = table!();

	if dark_background_color_scheme {
		table.add_row(row![
			bFG->"Total",
			FG->amount_to_hr_string(wallet_info.total, false)
		]);
		// Only dispay "Immature Coinbase" if we have related outputs in the wallet.
		// This row just introduces confusion if the wallet does not receive coinbase rewards.
		if wallet_info.amount_immature > 0 {
			table.add_row(row![
				bFY->format!("Immature Coinbase (< {})", global::coinbase_maturity()),
				FY->amount_to_hr_string(wallet_info.amount_immature, false)
			]);
		}
		table.add_row(row![
			bFY->format!("Awaiting Confirmation (< {})", wallet_info.minimum_confirmations),
			FY->amount_to_hr_string(wallet_info.amount_awaiting_confirmation, false)
		]);
		table.add_row(row![
			Fr->"Locked by previous transaction",
			Fr->amount_to_hr_string(wallet_info.amount_locked, false)
		]);
		table.add_row(row![
			Fw->"--------------------------------",
			Fw->"-------------"
		]);
		table.add_row(row![
			bFG->"Currently Spendable",
			FG->amount_to_hr_string(wallet_info.amount_currently_spendable, false)
		]);
	} else {
		table.add_row(row![
			bFG->"Total",
			FG->amount_to_hr_string(wallet_info.total, false)
		]);
		// Only dispay "Immature Coinbase" if we have related outputs in the wallet.
		// This row just introduces confusion if the wallet does not receive coinbase rewards.
		if wallet_info.amount_immature > 0 {
			table.add_row(row![
				bFB->format!("Immature Coinbase (< {})", global::coinbase_maturity()),
				FB->amount_to_hr_string(wallet_info.amount_immature, false)
			]);
		}
		table.add_row(row![
			bFB->format!("Awaiting Confirmation (< {})", wallet_info.minimum_confirmations),
			FB->amount_to_hr_string(wallet_info.amount_awaiting_confirmation, false)
		]);
		table.add_row(row![
			Fr->"Locked by previous transaction",
			Fr->amount_to_hr_string(wallet_info.amount_locked, false)
		]);
		table.add_row(row![
			Fw->"--------------------------------",
			Fw->"-------------"
		]);
		table.add_row(row![
			bFG->"Currently Spendable",
			FG->amount_to_hr_string(wallet_info.amount_currently_spendable, false)
		]);
	};
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
/// Display list of wallet accounts in a pretty way
pub fn accounts(acct_mappings: Vec<AcctPathMapping>) {
	println!("\n____ Wallet Accounts ____\n",);
	let mut table = table!();

	table.set_titles(row![
		mMG->"Name",
		bMG->"Parent BIP-32 Derivation Path",
	]);
	for m in acct_mappings {
		table.add_row(row![
			bFC->m.label,
			bGC->m.path.to_bip_32_string(),
		]);
	}
	table.set_format(*prettytable::format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
	table.printstd();
	println!();
}

/// Display transaction log messages
pub fn tx_messages(tx: &TxLogEntry, dark_background_color_scheme: bool) -> Result<(), Error> {
	let title = format!("Transaction Messages - Transaction '{}'", tx.id,);
	println!();
	let mut t = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	t.reset().unwrap();

	let msgs = match tx.messages.clone() {
		None => {
			writeln!(t, "{}", "None").unwrap();
			t.reset().unwrap();
			return Ok(());
		}
		Some(m) => m.clone(),
	};

	if msgs.messages.is_empty() {
		writeln!(t, "{}", "None").unwrap();
		t.reset().unwrap();
		return Ok(());
	}

	let mut table = table!();

	table.set_titles(row![
		bMG->"Participant Id",
		bMG->"Message",
		bMG->"Public Key",
		bMG->"Signature",
	]);

	let secp = util::static_secp_instance();
	let secp_lock = secp.lock();

	for m in msgs.messages {
		let id = format!("{}", m.id);
		let public_key = format!(
			"{}",
			util::to_hex(m.public_key.serialize_vec(&secp_lock, true).to_vec())
		);
		let message = match m.message {
			Some(m) => format!("{}", m),
			None => "None".to_owned(),
		};
		let message_sig = match m.message_sig {
			Some(s) => format!("{}", util::to_hex(s.serialize_der(&secp_lock))),
			None => "None".to_owned(),
		};
		if dark_background_color_scheme {
			table.add_row(row![
				bFC->id,
				bFC->message,
				bFC->public_key,
				bFB->message_sig,
			]);
		} else {
			table.add_row(row![
				bFD->id,
				bFb->message,
				bFD->public_key,
				bFB->message_sig,
			]);
		}
	}

	table.set_format(*prettytable::format::consts::FORMAT_NO_COLSEP);
	table.printstd();
	println!();

	Ok(())
}
