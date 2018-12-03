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

/// Argument parsing and error handling for wallet commands
use clap::ArgMatches;

use core::core;
use grin_wallet::command;
use std::path::Path;

pub fn parse_global_args(args: &ArgMatches) -> command::GlobalArgs {
	let account = match args.value_of("account") {
		None => {
			error!("Failed to read account.");
			std::process::exit(0);
		}
		Some(p) => p,
	};

	let mut show_spent = false;
	if args.is_present("show_spent") {
		show_spent = true;
	}

	command::GlobalArgs {
		account: account.to_owned(),
		show_spent: show_spent,
	}
}

pub fn parse_account_args(account_args: &ArgMatches) -> command::AccountArgs {
	let create = match account_args.value_of("create") {
		None => None,
		Some(s) => Some(s.to_owned()),
	};
	command::AccountArgs { create: create }
}

pub fn parse_send_args(send_args: &ArgMatches) -> command::SendArgs {
	let amount = send_args.value_of("amount").ok_or_else(|| {
		println!("Amount to send required");
		std::process::exit(0);
	});
	let amount = core::amount_from_hr_string(amount.unwrap()).map_err(|e| {
		println!(
			"Could not parse amount as a number with optional decimal point. e={:?}",
			e
		);
	});
	let message = match send_args.is_present("message") {
		true => Some(send_args.value_of("message").unwrap().to_owned()),
		false => None,
	};
	let minimum_confirmations: u64 = send_args
		.value_of("minimum_confirmations")
		.ok_or_else(|| {
			println!("Minimum confirmations to send required");
		}).and_then(|v| {
			v.parse().map_err(|e| {
				println!(
					"Could not parse minimum_confirmations as a whole number. e={:?}",
					e
				);
				std::process::exit(0);
			})
		}).unwrap();
	let selection_strategy = send_args
		.value_of("selection_strategy")
		.ok_or_else(|| {
			println!("Selection strategy required");
			std::process::exit(0);
		}).unwrap();
	let method = send_args
		.value_of("method")
		.ok_or_else(|| {
			println!("Payment method required");
			std::process::exit(0);
		}).unwrap();
	let dest = {
		if method == "self" {
			match send_args.value_of("dest") {
				Some(d) => d,
				None => "default",
			}
		} else {
			send_args
				.value_of("dest")
				.ok_or_else(|| {
					println!("Destination wallet address required");
					std::process::exit(0);
				}).unwrap()
		}
	};
	if method == "http" && !dest.starts_with("http://") && !dest.starts_with("https://") {
		println!(
			"HTTP Destination should start with http://: or https://: {}",
			dest,
		);
		std::process::exit(0);
	}
	let change_outputs = send_args
		.value_of("change_outputs")
		.ok_or_else(|| {
			println!("Change outputs required");
			std::process::exit(0);
		}).and_then(|v| {
			v.parse().map_err(|e| {
				println!("Failed to parse number of change outputs. e={:?}", e);
				std::process::exit(0);
			})
		});
	let fluff = send_args.is_present("fluff");
	let max_outputs = 500;
	command::SendArgs {
		amount: amount.unwrap(),
		message: message,
		minimum_confirmations: minimum_confirmations,
		selection_strategy: selection_strategy.to_owned(),
		method: method.to_owned(),
		dest: dest.to_owned(),
		change_outputs: change_outputs.unwrap(),
		fluff: fluff,
		max_outputs: max_outputs,
	}
}

pub fn parse_receive_args(receive_args: &ArgMatches) -> command::ReceiveArgs {
	let message = match receive_args.is_present("message") {
		true => Some(receive_args.value_of("message").unwrap().to_owned()),
		false => None,
	};
	let tx_file = receive_args
		.value_of("input")
		.ok_or_else(|| {
			println!("Transaction file required");
			std::process::exit(0);
		}).unwrap();
	if !Path::new(&tx_file).is_file() {
		println!("File {} not found.", &tx_file);
		std::process::exit(0);
	}
	command::ReceiveArgs {
		input: tx_file.to_owned(),
		message: message,
	}
}

pub fn parse_finalize_args(args: &ArgMatches) -> command::FinalizeArgs {
	let fluff = args.is_present("fluff");
	let tx_file = args
		.value_of("input")
		.ok_or_else(|| {
			println!("Receiver's transaction file required");
			std::process::exit(0);
		}).unwrap();
	if !Path::new(&tx_file).is_file() {
		println!("File {} not found.", tx_file);
		std::process::exit(0);
	}
	command::FinalizeArgs {
		input: tx_file.to_owned(),
		fluff: fluff,
	}
}

pub fn parse_info_args(args: &ArgMatches) -> command::InfoArgs {
	let minimum_confirmations = args
		.value_of("minimum_confirmations")
		.ok_or_else(|| {
			println!("Minimum confirmations required");
		}).and_then(|v| {
			v.parse().map_err(|e| {
				println!(
					"Could not parse minimum_confirmations as a whole number. e={:?}",
					e
				);
				std::process::exit(0);
			})
		});
	command::InfoArgs {
		minimum_confirmations: minimum_confirmations.unwrap(),
	}
}

pub fn parse_txs_args(args: &ArgMatches) -> command::TxsArgs {
	let tx_id = match args.value_of("id") {
		None => None,
		Some(tx) => match tx.parse() {
			Ok(t) => Some(t),
			Err(_) => {
				println!("Unable to parse argument 'id' as a number");
				std::process::exit(0);
			}
		},
	};
	command::TxsArgs { id: tx_id }
}

pub fn parse_repost_args(args: &ArgMatches) -> command::RepostArgs {
	let tx_id = args
		.value_of("id")
		.ok_or_else(|| {
			println!("Transaction of a completed but unconfirmed transaction required (specify with --id=[id])");
			std::process::exit(0);
		}).and_then(|v|{
		v.parse().map_err(|e| {
			println!(
				"Unable to parse argument 'id' as a number. e={:?}",
				e
			);
			std::process::exit(0);
		})});

	let fluff = args.is_present("fluff");
	let dump_file = match args.value_of("dumpfile") {
		None => None,
		Some(d) => Some(d.to_owned()),
	};
	command::RepostArgs {
		id: tx_id.unwrap(),
		dump_file: dump_file,
		fluff: fluff,
	}
}

pub fn parse_cancel_args(args: &ArgMatches) -> command::CancelArgs {
	let mut tx_id_string = "";
	let tx_id = match args.value_of("id") {
		None => None,
		Some(tx) => match tx.parse() {
			Ok(t) => {
				tx_id_string = tx;
				Some(t)
			}
			Err(e) => {
				println!("Could not parse id parameter. e={:?}", e,);
				std::process::exit(0);
			}
		},
	};
	let tx_slate_id = match args.value_of("txid") {
		None => None,
		Some(tx) => match tx.parse() {
			Ok(t) => {
				tx_id_string = tx;
				Some(t)
			}
			Err(e) => {
				println!("Could not parse txid parameter. e={:?}", e,);
				std::process::exit(0);
			}
		},
	};
	if (tx_id.is_none() && tx_slate_id.is_none()) || (tx_id.is_some() && tx_slate_id.is_some()) {
		println!("'id' (-i) or 'txid' (-t) argument is required.");
		std::process::exit(0);
	}
	command::CancelArgs {
		tx_id: tx_id,
		tx_slate_id: tx_slate_id,
		tx_id_string: tx_id_string.to_owned(),
	}
}
