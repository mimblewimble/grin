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
use failure::Fail;

use api::TLSConfig;
use core::core;
use grin_wallet::{self, command, WalletConfig, WalletSeed};
use std::path::Path;
use util::file::get_first_line;

/// Simple error definition, just so we can return errors from all commands
/// and let the caller figure out what to do
#[derive(Clone, Eq, PartialEq, Debug, Fail)]
pub enum Error {
	#[fail(display = "Invalid Arguments: {}", _0)]
	ArgumentError(String),
}

pub fn prompt_password(password: &Option<String>) -> String {
	match password {
		None => {
			println!("Temporary note:");
			println!(
				"If this is your first time running your wallet since BIP32 (word lists) \
				 were implemented, your seed will be converted to \
				 the new format. Please ensure the provided password is correct."
			);
			println!("If this goes wrong, your old 'wallet.seed' file has been saved as 'wallet.seed.bak' \
			Rename this file to back to `wallet.seed` and try again");
			rpassword::prompt_password_stdout("Password: ").unwrap()
		}
		Some(p) => p.to_owned(),
	}
}

fn prompt_password_confirm() -> String {
	let first = rpassword::prompt_password_stdout("Password: ").unwrap();
	let second = rpassword::prompt_password_stdout("Confirm Password: ").unwrap();
	if first != second {
		println!("Passwords do not match");
		std::process::exit(0);
	}
	first
}

// instantiate wallet (needed by most functions)

pub fn instantiate_wallet(
	config: WalletConfig,
	g_args: &command::GlobalArgs,
) -> Result<command::WalletRef, Error> {
	let passphrase = prompt_password(&g_args.password);
	let res = grin_wallet::instantiate_wallet(
		config.clone(),
		&passphrase,
		&g_args.account,
		g_args.node_api_secret.clone(),
	);
	match res {
		Ok(p) => Ok(p),
		Err(e) => {
			let msg = {
				match e.kind() {
					grin_wallet::ErrorKind::Encryption => {
						format!("Error decrypting wallet seed (check provided password)")
					}
					_ => format!("Error instantiating wallet: {}", e),
				}
			};
			Err(Error::ArgumentError(msg))
		}
	}
}

// parses a required value, or throws error with message otherwise
fn parse_required<'a>(args: &'a ArgMatches, name: &str) -> Result<&'a str, Error> {
	let arg = args.value_of(name);
	match arg {
		Some(ar) => Ok(ar),
		None => {
			let msg = format!("Value for argument '{}' is required in this context", name,);
			Err(Error::ArgumentError(msg))
		}
	}
}

// parses a number, or throws error with message otherwise
fn parse_u64(arg: &str, name: &str) -> Result<u64, Error> {
	let val = arg.parse::<u64>();
	match val {
		Ok(v) => Ok(v),
		Err(e) => {
			let msg = format!("Could not parse {} as a whole number. e={}", name, e);
			Err(Error::ArgumentError(msg))
		}
	}
}

pub fn parse_global_args(
	config: &WalletConfig,
	args: &ArgMatches,
) -> Result<command::GlobalArgs, Error> {
	let account = parse_required(args, "account")?;
	let mut show_spent = false;
	if args.is_present("show_spent") {
		show_spent = true;
	}
	let node_api_secret = get_first_line(config.node_api_secret_path.clone());
	let password = match args.value_of("pass") {
		None => None,
		Some(p) => Some(p.to_owned()),
	};

	let tls_conf = match config.tls_certificate_file.clone() {
		None => None,
		Some(file) => {
			let key = match config.tls_certificate_key.clone() {
				Some(k) => k,
				None => {
					let msg = format!("Private key for certificate is not set");
					return Err(Error::ArgumentError(msg));
				}
			};
			Some(TLSConfig::new(file, key))
		}
	};

	Ok(command::GlobalArgs {
		account: account.to_owned(),
		show_spent: show_spent,
		node_api_secret: node_api_secret,
		password: password,
		tls_conf: tls_conf,
	})
}

pub fn parse_init_args(
	config: &WalletConfig,
	args: &ArgMatches,
) -> Result<command::InitArgs, Error> {
	if let Err(e) = WalletSeed::seed_file_exists(config) {
		let msg = format!("Not creating wallet - {}", e.inner);
		return Err(Error::ArgumentError(msg));
	}
	let list_length = match args.is_present("short_wordlist") {
		false => 32,
		true => 16,
	};
	println!("Please enter a password for your new wallet");
	let password = prompt_password_confirm();
	Ok(command::InitArgs {
		list_length: list_length,
		password: password,
		config: config.clone(),
	})
}

pub fn parse_recover_args(
	g_args: &command::GlobalArgs,
	args: &ArgMatches,
) -> Result<command::RecoverArgs, Error> {
	let (passphrase, recovery_phrase) = {
		match args.value_of("recovery_phrase") {
			None => (prompt_password(&g_args.password), None),
			Some(l) => {
				if WalletSeed::from_mnemonic(l).is_err() {
					let msg = format!("Recovery word phrase is invalid");
					return Err(Error::ArgumentError(msg));
				}
				println!("Please provide a new password for the recovered wallet");
				(prompt_password_confirm(), Some(l.to_owned()))
			}
		}
	};
	Ok(command::RecoverArgs {
		passphrase: passphrase,
		recovery_phrase: recovery_phrase,
	})
}

pub fn parse_listen_args(
	config: &mut WalletConfig,
	g_args: &mut command::GlobalArgs,
	args: &ArgMatches,
) -> Result<(), Error> {
	// listen args
	let pass = match args.value_of("pass") {
		Some(p) => Some(p.to_owned()),
		None => Some(prompt_password(&None)),
	};
	g_args.password = pass;
	if let Some(port) = args.value_of("port") {
		config.api_listen_port = port.parse().unwrap();
	}
	Ok(())
}

pub fn parse_account_args(account_args: &ArgMatches) -> Result<command::AccountArgs, Error> {
	let create = match account_args.value_of("create") {
		None => None,
		Some(s) => Some(s.to_owned()),
	};
	Ok(command::AccountArgs { create: create })
}

pub fn parse_send_args(args: &ArgMatches) -> Result<command::SendArgs, Error> {
	// amount
	let amount = parse_required(args, "amount")?;
	let amount = core::amount_from_hr_string(amount);
	let amount = match amount {
		Ok(a) => a,
		Err(e) => {
			let msg = format!(
				"Could not parse amount as a number with optional decimal point. e={}",
				e
			);
			return Err(Error::ArgumentError(msg));
		}
	};

	// message
	let message = match args.is_present("message") {
		true => Some(args.value_of("message").unwrap().to_owned()),
		false => None,
	};

	// minimum_confirmations
	let min_c = parse_required(args, "minimum_confirmations")?;
	let min_c = parse_u64(min_c, "minimum_confirmations")?;

	// selection_strategy
	let selection_strategy = parse_required(args, "selection_strategy")?;

	// method
	let method = parse_required(args, "method")?;

	// dest
	let dest = {
		if method == "self" {
			match args.value_of("dest") {
				Some(d) => d,
				None => "default",
			}
		} else {
			parse_required(args, "dest")?
		}
	};
	if method == "http" && !dest.starts_with("http://") && !dest.starts_with("https://") {
		let msg = format!(
			"HTTP Destination should start with http://: or https://: {}",
			dest,
		);
		return Err(Error::ArgumentError(msg));
	}

	// change_outputs
	let change_outputs = parse_required(args, "change_outputs")?;
	let change_outputs = parse_u64(change_outputs, "change_outputs")? as usize;

	// fluff
	let fluff = args.is_present("fluff");

	// max_outputs
	let max_outputs = 500;

	Ok(command::SendArgs {
		amount: amount,
		message: message,
		minimum_confirmations: min_c,
		selection_strategy: selection_strategy.to_owned(),
		method: method.to_owned(),
		dest: dest.to_owned(),
		change_outputs: change_outputs,
		fluff: fluff,
		max_outputs: max_outputs,
	})
}

pub fn parse_receive_args(receive_args: &ArgMatches) -> Result<command::ReceiveArgs, Error> {
	// message
	let message = match receive_args.is_present("message") {
		true => Some(receive_args.value_of("message").unwrap().to_owned()),
		false => None,
	};

	// input
	let tx_file = parse_required(receive_args, "input")?;

	// validate input
	if !Path::new(&tx_file).is_file() {
		let msg = format!("File {} not found.", &tx_file);
		return Err(Error::ArgumentError(msg));
	}

	Ok(command::ReceiveArgs {
		input: tx_file.to_owned(),
		message: message,
	})
}

pub fn parse_finalize_args(args: &ArgMatches) -> Result<command::FinalizeArgs, Error> {
	let fluff = args.is_present("fluff");
	let tx_file = parse_required(args, "input")?;

	if !Path::new(&tx_file).is_file() {
		let msg = format!("File {} not found.", tx_file);
		return Err(Error::ArgumentError(msg));
	}
	Ok(command::FinalizeArgs {
		input: tx_file.to_owned(),
		fluff: fluff,
	})
}

pub fn parse_info_args(args: &ArgMatches) -> Result<command::InfoArgs, Error> {
	// minimum_confirmations
	let mc = parse_required(args, "minimum_confirmations")?;
	let mc = parse_u64(mc, "minimum_confirmations")?;
	Ok(command::InfoArgs {
		minimum_confirmations: mc,
	})
}

pub fn parse_txs_args(args: &ArgMatches) -> Result<command::TxsArgs, Error> {
	let tx_id = match args.value_of("id") {
		None => None,
		Some(tx) => Some(parse_u64(tx, "id")? as u32),
	};
	Ok(command::TxsArgs { id: tx_id })
}

pub fn parse_repost_args(args: &ArgMatches) -> Result<command::RepostArgs, Error> {
	let tx_id = match args.value_of("id") {
		None => None,
		Some(tx) => Some(parse_u64(tx, "id")? as u32),
	};

	let fluff = args.is_present("fluff");
	let dump_file = match args.value_of("dumpfile") {
		None => None,
		Some(d) => Some(d.to_owned()),
	};

	Ok(command::RepostArgs {
		id: tx_id.unwrap(),
		dump_file: dump_file,
		fluff: fluff,
	})
}

pub fn parse_cancel_args(args: &ArgMatches) -> Result<command::CancelArgs, Error> {
	let mut tx_id_string = "";
	let tx_id = match args.value_of("id") {
		None => None,
		Some(tx) => Some(parse_u64(tx, "id")? as u32),
	};
	let tx_slate_id = match args.value_of("txid") {
		None => None,
		Some(tx) => match tx.parse() {
			Ok(t) => {
				tx_id_string = tx;
				Some(t)
			}
			Err(e) => {
				let msg = format!("Could not parse txid parameter. e={}", e);
				return Err(Error::ArgumentError(msg));
			}
		},
	};
	if (tx_id.is_none() && tx_slate_id.is_none()) || (tx_id.is_some() && tx_slate_id.is_some()) {
		let msg = format!("'id' (-i) or 'txid' (-t) argument is required.");
		return Err(Error::ArgumentError(msg));
	}
	Ok(command::CancelArgs {
		tx_id: tx_id,
		tx_slate_id: tx_slate_id,
		tx_id_string: tx_id_string.to_owned(),
	})
}
