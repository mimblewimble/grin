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

use crate::api::TLSConfig;
use crate::util::file::get_first_line;
use crate::util::Mutex;
/// Argument parsing and error handling for wallet commands
use clap::ArgMatches;
use failure::Fail;
use grin_core as core;
use grin_keychain as keychain;
use grin_wallet::{command, instantiate_wallet, NodeClient, WalletConfig, WalletInst, WalletSeed};
use grin_wallet::{Error, ErrorKind};
use linefeed::terminal::Signal;
use linefeed::{Interface, ReadResult};
use rpassword;
use std::path::Path;
use std::sync::Arc;

// define what to do on argument error
macro_rules! arg_parse {
	( $r:expr ) => {
		match $r {
			Ok(res) => res,
			Err(e) => {
				return Err(ErrorKind::ArgumentError(format!("{}", e)).into());
				}
			}
	};
}
/// Simple error definition, just so we can return errors from all commands
/// and let the caller figure out what to do
#[derive(Clone, Eq, PartialEq, Debug, Fail)]
pub enum ParseError {
	#[fail(display = "Invalid Arguments: {}", _0)]
	ArgumentError(String),
	#[fail(display = "Parsing IO error: {}", _0)]
	IOError(String),
	#[fail(display = "User Cancelled")]
	CancelledError,
}

impl From<std::io::Error> for ParseError {
	fn from(e: std::io::Error) -> ParseError {
		ParseError::IOError(format!("{}", e))
	}
}

pub fn prompt_password(password: &Option<String>) -> String {
	match password {
		None => rpassword::prompt_password_stdout("Password: ").unwrap(),
		Some(p) => p.to_owned(),
	}
}

fn prompt_password_confirm() -> String {
	let mut first = String::from("first");
	let mut second = String::from("second");
	while first != second {
		first = rpassword::prompt_password_stdout("Password: ").unwrap();
		second = rpassword::prompt_password_stdout("Confirm Password: ").unwrap();
	}
	first
}

fn prompt_recovery_phrase() -> Result<String, ParseError> {
	let interface = Arc::new(Interface::new("recover")?);
	let mut phrase = String::new();
	interface.set_report_signal(Signal::Interrupt, true);
	interface.set_prompt("phrase> ")?;
	loop {
		println!("Please enter your recovery phrase:");
		let res = interface.read_line()?;
		match res {
			ReadResult::Eof => break,
			ReadResult::Signal(sig) => {
				if sig == Signal::Interrupt {
					interface.cancel_read_line()?;
					return Err(ParseError::CancelledError);
				}
			}
			ReadResult::Input(line) => {
				if WalletSeed::from_mnemonic(&line).is_ok() {
					phrase = line;
					break;
				} else {
					println!();
					println!("Recovery word phrase is invalid.");
					println!();
					interface.set_buffer(&line)?;
				}
			}
		}
	}
	Ok(phrase)
}

// instantiate wallet (needed by most functions)

pub fn inst_wallet(
	config: WalletConfig,
	g_args: &command::GlobalArgs,
	node_client: impl NodeClient + 'static,
) -> Result<Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>, ParseError> {
	let passphrase = prompt_password(&g_args.password);
	let res = instantiate_wallet(config.clone(), node_client, &passphrase, &g_args.account);
	match res {
		Ok(p) => Ok(p),
		Err(e) => {
			let msg = {
				match e.kind() {
					ErrorKind::Encryption => {
						format!("Error decrypting wallet seed (check provided password)")
					}
					_ => format!("Error instantiating wallet: {}", e),
				}
			};
			Err(ParseError::ArgumentError(msg))
		}
	}
}

// parses a required value, or throws error with message otherwise
fn parse_required<'a>(args: &'a ArgMatches, name: &str) -> Result<&'a str, ParseError> {
	let arg = args.value_of(name);
	match arg {
		Some(ar) => Ok(ar),
		None => {
			let msg = format!("Value for argument '{}' is required in this context", name,);
			Err(ParseError::ArgumentError(msg))
		}
	}
}

// parses a number, or throws error with message otherwise
fn parse_u64(arg: &str, name: &str) -> Result<u64, ParseError> {
	let val = arg.parse::<u64>();
	match val {
		Ok(v) => Ok(v),
		Err(e) => {
			let msg = format!("Could not parse {} as a whole number. e={}", name, e);
			Err(ParseError::ArgumentError(msg))
		}
	}
}

pub fn parse_global_args(
	config: &WalletConfig,
	args: &ArgMatches,
) -> Result<command::GlobalArgs, ParseError> {
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
					return Err(ParseError::ArgumentError(msg));
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
	g_args: &command::GlobalArgs,
	args: &ArgMatches,
) -> Result<command::InitArgs, ParseError> {
	if let Err(e) = WalletSeed::seed_file_exists(config) {
		let msg = format!("Not creating wallet - {}", e.inner);
		return Err(ParseError::ArgumentError(msg));
	}
	let list_length = match args.is_present("short_wordlist") {
		false => 32,
		true => 16,
	};
	println!("Please enter a password for your new wallet");
	let password = match g_args.password.clone() {
		Some(p) => p,
		None => prompt_password_confirm(),
	};
	Ok(command::InitArgs {
		list_length: list_length,
		password: password,
		config: config.clone(),
	})
}

pub fn parse_recover_args(
	g_args: &command::GlobalArgs,
	args: &ArgMatches,
) -> Result<command::RecoverArgs, ParseError> {
	let (passphrase, recovery_phrase) = {
		match args.is_present("display") {
			true => (prompt_password(&g_args.password), None),
			false => {
				let phrase = prompt_recovery_phrase()?;
				println!("Please provide a new password for the recovered wallet");
				(prompt_password_confirm(), Some(phrase.to_owned()))
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
) -> Result<command::ListenArgs, ParseError> {
	// listen args
	let pass = match g_args.password.clone() {
		Some(p) => Some(p.to_owned()),
		None => Some(prompt_password(&None)),
	};
	g_args.password = pass;
	if let Some(port) = args.value_of("port") {
		config.api_listen_port = port.parse().unwrap();
	}
	let method = parse_required(args, "method")?;
	Ok(command::ListenArgs {
		method: method.to_owned(),
	})
}

pub fn parse_account_args(account_args: &ArgMatches) -> Result<command::AccountArgs, ParseError> {
	let create = match account_args.value_of("create") {
		None => None,
		Some(s) => Some(s.to_owned()),
	};
	Ok(command::AccountArgs { create: create })
}

pub fn parse_send_args(args: &ArgMatches) -> Result<command::SendArgs, ParseError> {
	// amount
	let amount = parse_required(args, "amount")?;
	let amount = core::core::amount_from_hr_string(amount);
	let amount = match amount {
		Ok(a) => a,
		Err(e) => {
			let msg = format!(
				"Could not parse amount as a number with optional decimal point. e={}",
				e
			);
			return Err(ParseError::ArgumentError(msg));
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
		return Err(ParseError::ArgumentError(msg));
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

pub fn parse_receive_args(receive_args: &ArgMatches) -> Result<command::ReceiveArgs, ParseError> {
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
		return Err(ParseError::ArgumentError(msg));
	}

	Ok(command::ReceiveArgs {
		input: tx_file.to_owned(),
		message: message,
	})
}

pub fn parse_finalize_args(args: &ArgMatches) -> Result<command::FinalizeArgs, ParseError> {
	let fluff = args.is_present("fluff");
	let tx_file = parse_required(args, "input")?;

	if !Path::new(&tx_file).is_file() {
		let msg = format!("File {} not found.", tx_file);
		return Err(ParseError::ArgumentError(msg));
	}
	Ok(command::FinalizeArgs {
		input: tx_file.to_owned(),
		fluff: fluff,
	})
}

pub fn parse_info_args(args: &ArgMatches) -> Result<command::InfoArgs, ParseError> {
	// minimum_confirmations
	let mc = parse_required(args, "minimum_confirmations")?;
	let mc = parse_u64(mc, "minimum_confirmations")?;
	Ok(command::InfoArgs {
		minimum_confirmations: mc,
	})
}

pub fn parse_txs_args(args: &ArgMatches) -> Result<command::TxsArgs, ParseError> {
	let tx_id = match args.value_of("id") {
		None => None,
		Some(tx) => Some(parse_u64(tx, "id")? as u32),
	};
	Ok(command::TxsArgs { id: tx_id })
}

pub fn parse_repost_args(args: &ArgMatches) -> Result<command::RepostArgs, ParseError> {
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

pub fn parse_cancel_args(args: &ArgMatches) -> Result<command::CancelArgs, ParseError> {
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
				return Err(ParseError::ArgumentError(msg));
			}
		},
	};
	if (tx_id.is_none() && tx_slate_id.is_none()) || (tx_id.is_some() && tx_slate_id.is_some()) {
		let msg = format!("'id' (-i) or 'txid' (-t) argument is required.");
		return Err(ParseError::ArgumentError(msg));
	}
	Ok(command::CancelArgs {
		tx_id: tx_id,
		tx_slate_id: tx_slate_id,
		tx_id_string: tx_id_string.to_owned(),
	})
}

pub fn wallet_command(
	wallet_args: &ArgMatches,
	mut wallet_config: WalletConfig,
	mut node_client: impl NodeClient + 'static,
) -> Result<String, Error> {
	if let Some(t) = wallet_config.chain_type.clone() {
		core::global::set_mining_mode(t);
	}

	if wallet_args.is_present("external") {
		wallet_config.api_listen_interface = "0.0.0.0".to_string();
	}

	if let Some(dir) = wallet_args.value_of("dir") {
		wallet_config.data_file_dir = dir.to_string().clone();
	}

	if let Some(sa) = wallet_args.value_of("api_server_address") {
		wallet_config.check_node_api_http_addr = sa.to_string().clone();
	}

	let global_wallet_args = arg_parse!(parse_global_args(&wallet_config, &wallet_args));

	node_client.set_node_url(&wallet_config.check_node_api_http_addr);
	node_client.set_node_api_secret(global_wallet_args.node_api_secret.clone());

	// closure to instantiate wallet as needed by each subcommand
	let inst_wallet = || {
		let res = inst_wallet(wallet_config.clone(), &global_wallet_args, node_client);
		res.unwrap_or_else(|e| {
			println!("{}", e);
			std::process::exit(1);
		})
	};

	let res = match wallet_args.subcommand() {
		("init", Some(args)) => {
			let a = arg_parse!(parse_init_args(&wallet_config, &global_wallet_args, &args));
			command::init(&global_wallet_args, a)
		}
		("recover", Some(args)) => {
			let a = arg_parse!(parse_recover_args(&global_wallet_args, &args));
			command::recover(&wallet_config, a)
		}
		("listen", Some(args)) => {
			let mut c = wallet_config.clone();
			let mut g = global_wallet_args.clone();
			let a = arg_parse!(parse_listen_args(&mut c, &mut g, &args));
			command::listen(&wallet_config, &a, &g)
		}
		("owner_api", Some(_)) => {
			let mut g = global_wallet_args.clone();
			g.tls_conf = None;
			command::owner_api(inst_wallet(), &wallet_config, &g)
		}
		("web", Some(_)) => command::owner_api(inst_wallet(), &wallet_config, &global_wallet_args),
		("account", Some(args)) => {
			let a = arg_parse!(parse_account_args(&args));
			command::account(inst_wallet(), a)
		}
		("send", Some(args)) => {
			let a = arg_parse!(parse_send_args(&args));
			command::send(inst_wallet(), a)
		}
		("receive", Some(args)) => {
			let a = arg_parse!(parse_receive_args(&args));
			command::receive(inst_wallet(), &global_wallet_args, a)
		}
		("finalize", Some(args)) => {
			let a = arg_parse!(parse_finalize_args(&args));
			command::finalize(inst_wallet(), a)
		}
		("info", Some(args)) => {
			let a = arg_parse!(parse_info_args(&args));
			command::info(
				inst_wallet(),
				&global_wallet_args,
				a,
				wallet_config.dark_background_color_scheme.unwrap_or(true),
			)
		}
		("outputs", Some(_)) => command::outputs(
			inst_wallet(),
			&global_wallet_args,
			wallet_config.dark_background_color_scheme.unwrap_or(true),
		),
		("txs", Some(args)) => {
			let a = arg_parse!(parse_txs_args(&args));
			command::txs(
				inst_wallet(),
				&global_wallet_args,
				a,
				wallet_config.dark_background_color_scheme.unwrap_or(true),
			)
		}
		("repost", Some(args)) => {
			let a = arg_parse!(parse_repost_args(&args));
			command::repost(inst_wallet(), a)
		}
		("cancel", Some(args)) => {
			let a = arg_parse!(parse_cancel_args(&args));
			command::cancel(inst_wallet(), a)
		}
		("restore", Some(_)) => command::restore(inst_wallet()),
		("check_repair", Some(_)) => command::check_repair(inst_wallet()),
		_ => {
			let msg = format!("Unknown wallet command, use 'grin help wallet' for details");
			return Err(ErrorKind::ArgumentError(msg).into());
		}
	};
	if let Err(e) = res {
		Err(e)
	} else {
		Ok(wallet_args.subcommand().0.to_owned())
	}
}
