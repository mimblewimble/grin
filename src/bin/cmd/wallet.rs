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

use clap::ArgMatches;
use rpassword;
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use super::wallet_args;
use api::TLSConfig;
use config::GlobalWalletConfig;
use core::global;
use grin_wallet::libwallet::ErrorKind;
use grin_wallet::{self, controller};
use grin_wallet::{
	command, instantiate_wallet, HTTPNodeClient, HTTPWalletCommAdapter, LMDBBackend, WalletConfig,
	WalletSeed,
};
use keychain;
use servers::start_webwallet_server;
use util::file::get_first_line;

// define what to do on argument error
macro_rules! arg_parse {
	( $r:expr ) => {
		match $r {
			Ok(res) => res,
			Err(e) => {
				println!("{}", e);
				return 0;
				}
			}
	};
}

pub fn _init_wallet_seed(wallet_config: WalletConfig, password: &str) {
	if let Err(_) = WalletSeed::from_file(&wallet_config, password) {
		WalletSeed::init_file(&wallet_config, 32, password)
			.expect("Failed to create wallet seed file.");
	};
}

pub fn seed_exists(wallet_config: WalletConfig) -> bool {
	let mut data_file_dir = PathBuf::new();
	data_file_dir.push(wallet_config.data_file_dir);
	data_file_dir.push(grin_wallet::SEED_FILE);
	if data_file_dir.exists() {
		true
	} else {
		false
	}
}

pub fn wallet_command(wallet_args: &ArgMatches, config: GlobalWalletConfig) -> i32 {
	// just get defaults from the global config
	let mut wallet_config = config.members.unwrap().wallet;

	if let Some(t) = wallet_config.chain_type.clone() {
		global::set_mining_mode(t);
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

	let node_api_secret = get_first_line(wallet_config.node_api_secret_path.clone());

	let global_wallet_args =
		arg_parse!(wallet_args::parse_global_args(&wallet_config, &wallet_args));

	/*
	// Decrypt the seed from the seed file and derive the keychain.
	// Generate the initial wallet seed if we are running "wallet init".
	if let ("init", Some(r)) = wallet_args.subcommand() {
		let a = arg_parse!(wallet_args::parse_init_args(&wallet_config, &args));
		command::init(wallet.clone(), a)
		thread::sleep(Duration::from_millis(200));
		// we are done here with creating the wallet, so just return
		return 0;
	}

	// Recover a seed from a recovery phrase
	if let ("recover", Some(r)) = wallet_args.subcommand() {
		if !r.is_present("recovery_phrase") {
			// only needed to display phrase
			let passphrase = prompt_password(wallet_args);
			let seed = match WalletSeed::from_file(&wallet_config, &passphrase) {
				Ok(s) => s,
				Err(e) => {
					println!("Can't open wallet seed file (check password): {}", e);
					std::process::exit(0);
				}
			};
			let _ = seed.show_recovery_phrase();
			std::process::exit(0);
		}
		let word_list = match r.value_of("recovery_phrase") {
			Some(w) => w,
			None => {
				println!("Recovery word phrase must be provided (in quotes)");
				std::process::exit(0);
			}
		};
		// check word list is okay before asking for password
		if WalletSeed::from_mnemonic(word_list).is_err() {
			println!("Recovery word phrase is invalid");
			std::process::exit(0);
		}
		println!("Please provide a new password for the recovered wallet");
		let passphrase = prompt_password_confirm();
		let res = WalletSeed::recover_from_phrase(&wallet_config, word_list, &passphrase);
		if let Err(e) = res {
			thread::sleep(Duration::from_millis(200));
			error!("Error recovering seed with list '{}' - {}", word_list, e);
			return 0;
		}

		thread::sleep(Duration::from_millis(200));
		return 0;
	}

	let account = match wallet_args.value_of("account") {
		None => {
			error!("Failed to read account.");
			return 1;
		}
		Some(p) => p,
	};

	// all further commands always need a password
	let passphrase = prompt_password(wallet_args);

	// Handle listener startup commands
	{
		let api_secret = get_first_line(wallet_config.api_secret_path.clone());

		let tls_conf = match wallet_config.tls_certificate_file.clone() {
			None => None,
			Some(file) => Some(TLSConfig::new(
				file,
				wallet_config
					.tls_certificate_key
					.clone()
					.unwrap_or_else(|| {
						panic!("Private key for certificate is not set");
					}),
			)),
		};
		match wallet_args.subcommand() {
			("listen", Some(listen_args)) => {
				if let Some(port) = listen_args.value_of("port") {
					wallet_config.api_listen_port = port.parse().unwrap();
				}
				let mut params = HashMap::new();
				params.insert(
					"api_listen_addr".to_owned(),
					wallet_config.api_listen_addr(),
				);
				if let Some(t) = tls_conf {
					params.insert("certificate".to_owned(), t.certificate);
					params.insert("private_key".to_owned(), t.private_key);
				}
				let adapter = HTTPWalletCommAdapter::new();
				adapter
					.listen(
						params,
						wallet_config.clone(),
						&passphrase,
						account,
						node_api_secret.clone(),
					).unwrap_or_else(|e| {
						if e.kind() == ErrorKind::WalletSeedDecryption {
							println!("Error decrypting wallet seed (check provided password)");
							std::process::exit(0);
						}
						panic!(
							"Error creating wallet listener: {:?} Config: {:?}",
							e, wallet_config
						);
					});
			}
			("owner_api", Some(_api_args)) => {
				let wallet = instantiate_wallet(
					wallet_config.clone(),
					&passphrase,
					account,
					node_api_secret.clone(),
				).unwrap_or_else(|e| {
					if e.kind() == grin_wallet::ErrorKind::Encryption {
						println!("Error decrypting wallet seed (check provided password)");
						std::process::exit(0);
					}
					panic!(
						"Error creating wallet listener: {:?} Config: {:?}",
						e, wallet_config
					);
				});
				// TLS is disabled because we bind to localhost
				controller::owner_listener(wallet.clone(), "127.0.0.1:13420", api_secret, None)
					.unwrap_or_else(|e| {
						panic!(
							"Error creating wallet api listener: {:?} Config: {:?}",
							e, wallet_config
						);
					});
			}
			("web", Some(_api_args)) => {
				let wallet = instantiate_wallet(
					wallet_config.clone(),
					&passphrase,
					account,
					node_api_secret.clone(),
				).unwrap_or_else(|e| {
					if e.kind() == grin_wallet::ErrorKind::Encryption {
						println!("Error decrypting wallet seed (check provided password)");
						std::process::exit(0);
					}
					panic!(
						"Error creating wallet listener: {:?} Config: {:?}",
						e, wallet_config
					);
				});
				// start owner listener and run static file server
				start_webwallet_server();
				controller::owner_listener(wallet.clone(), "127.0.0.1:13420", api_secret, tls_conf)
					.unwrap_or_else(|e| {
						panic!(
							"Error creating wallet api listener: {:?} Config: {:?}",
							e, wallet_config
						);
					});
			}
			_ => {}
		};
	}
	*/

	// closure to instantiate wallet as needed by each subcommand
	let inst_wallet = || {
		let res = wallet_args::instantiate_wallet(wallet_config.clone(), &global_wallet_args);
		res.unwrap_or_else(|e| {
			println!("{}", e);
			std::process::exit(0);
		})
	};

	let res = match wallet_args.subcommand() {
		("init", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_init_args(&wallet_config, &args));
			command::init(&global_wallet_args, a)
		}
		("recover", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_recover_args(&global_wallet_args, &args));
			command::recover(&wallet_config, a)
		}
		("account", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_account_args(&args));
			command::account(inst_wallet(), a)
		}
		("send", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_send_args(&args));
			command::send(inst_wallet(), a)
		}
		("receive", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_receive_args(&args));
			command::receive(inst_wallet(), &global_wallet_args, a)
		}
		("finalize", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_finalize_args(&args));
			command::finalize(inst_wallet(), a)
		}
		("info", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_info_args(&args));
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
			let a = arg_parse!(wallet_args::parse_txs_args(&args));
			command::txs(
				inst_wallet(),
				&global_wallet_args,
				a,
				wallet_config.dark_background_color_scheme.unwrap_or(true),
			)
		}
		("repost", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_repost_args(&args));
			command::repost(inst_wallet(), a)
		}
		("cancel", Some(args)) => {
			let a = arg_parse!(wallet_args::parse_cancel_args(&args));
			command::cancel(inst_wallet(), a)
		}
		("restore", Some(_)) => command::restore(inst_wallet()),
		_ => {
			println!("Unknown wallet command, use 'grin help wallet' for details");
			return 0;
		}
	};
	// we need to give log output a chance to catch up before exiting
	thread::sleep(Duration::from_millis(100));

	if let Err(e) = res {
		println!("Wallet command failed: {}", e);
		1
	} else {
		0
	}
}
