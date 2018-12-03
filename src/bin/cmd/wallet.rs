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

pub fn prompt_password(args: &ArgMatches) -> String {
	match args.value_of("pass") {
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

pub fn prompt_password_confirm() -> String {
	let first = rpassword::prompt_password_stdout("Password: ").unwrap();
	let second = rpassword::prompt_password_stdout("Confirm Password: ").unwrap();
	if first != second {
		println!("Passwords do not match");
		std::process::exit(0);
	}
	first
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

	let global_wallet_args = wallet_args::parse_global_args(&wallet_args);

	// Decrypt the seed from the seed file and derive the keychain.
	// Generate the initial wallet seed if we are running "wallet init".
	if let ("init", Some(r)) = wallet_args.subcommand() {
		if let Err(e) = WalletSeed::seed_file_exists(&wallet_config) {
			println!(
				"Not creating wallet - Wallet seed file already exists at {}",
				e.inner
			);
			return 0;
		}
		let list_length = match r.is_present("short_wordlist") {
			false => 32,
			true => 16,
		};
		println!("Please enter a password for your new wallet");
		let passphrase = prompt_password_confirm();
		WalletSeed::init_file(&wallet_config, list_length, &passphrase)
			.expect("Failed to init wallet seed file.");
		info!("Wallet seed file created");
		let client_n =
			HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, node_api_secret);
		let _: LMDBBackend<HTTPNodeClient, keychain::ExtKeychain> =
			LMDBBackend::new(wallet_config.clone(), &passphrase, client_n).unwrap_or_else(|e| {
				panic!(
					"Error creating DB for wallet: {} Config: {:?}",
					e, wallet_config
				);
			});
		info!("Wallet database backend created");
		// give logging thread a moment to catch up
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
			"Error instantiating wallet: {:?} Config: {:?}",
			e, wallet_config
		);
	});

	let res = match wallet_args.subcommand() {
		("account", Some(args)) => {
			command::account(wallet.clone(), wallet_args::parse_account_args(&args))
		}
		("send", Some(args)) => command::send(wallet.clone(), wallet_args::parse_send_args(&args)),
		("receive", Some(args)) => command::receive(
			wallet.clone(),
			&global_wallet_args,
			wallet_args::parse_receive_args(&args),
		),
		("finalize", Some(args)) => {
			command::finalize(wallet.clone(), wallet_args::parse_finalize_args(&args))
		}
		("info", Some(args)) => command::info(
			wallet.clone(),
			&global_wallet_args,
			wallet_args::parse_info_args(&args),
			wallet_config.dark_background_color_scheme.unwrap_or(true),
		),
		("outputs", Some(_)) => command::outputs(
			wallet.clone(),
			&global_wallet_args,
			wallet_config.dark_background_color_scheme.unwrap_or(true),
		),
		("txs", Some(args)) => command::txs(
			wallet.clone(),
			&global_wallet_args,
			wallet_args::parse_txs_args(&args),
			wallet_config.dark_background_color_scheme.unwrap_or(true),
		),
		("repost", Some(args)) => {
			command::repost(wallet.clone(), wallet_args::parse_repost_args(&args))
		}
		("cancel", Some(args)) => {
			command::cancel(wallet.clone(), wallet_args::parse_cancel_args(&args))
		}
		("restore", Some(_)) => command::restore(wallet.clone()),
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
