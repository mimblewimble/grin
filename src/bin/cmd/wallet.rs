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
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use config::GlobalWalletConfig;
use core::global;
use grin_wallet::{self, command, command_args, WalletConfig, WalletSeed};
use servers::start_webwallet_server;

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

	let global_wallet_args = arg_parse!(command_args::parse_global_args(
		&wallet_config,
		&wallet_args
	));

	// closure to instantiate wallet as needed by each subcommand
	let inst_wallet = || {
		let res = command_args::inst_wallet(wallet_config.clone(), &global_wallet_args);
		res.unwrap_or_else(|e| {
			println!("{}", e);
			std::process::exit(0);
		})
	};

	let res = match wallet_args.subcommand() {
		("init", Some(args)) => {
			let a = arg_parse!(command_args::parse_init_args(&wallet_config, &args));
			command::init(&global_wallet_args, a)
		}
		("recover", Some(args)) => {
			let a = arg_parse!(command_args::parse_recover_args(&global_wallet_args, &args));
			command::recover(&wallet_config, a)
		}
		("listen", Some(args)) => {
			let mut c = wallet_config.clone();
			let mut g = global_wallet_args.clone();
			arg_parse!(command_args::parse_listen_args(&mut c, &mut g, &args));
			command::listen(&wallet_config, &g)
		}
		("owner_api", Some(_)) => {
			let mut g = global_wallet_args.clone();
			g.tls_conf = None;
			command::owner_api(inst_wallet(), &g)
		}
		("web", Some(_)) => {
			start_webwallet_server();
			command::owner_api(inst_wallet(), &global_wallet_args)
		}
		("account", Some(args)) => {
			let a = arg_parse!(command_args::parse_account_args(&args));
			command::account(inst_wallet(), a)
		}
		("send", Some(args)) => {
			let a = arg_parse!(command_args::parse_send_args(&args));
			command::send(inst_wallet(), a)
		}
		("receive", Some(args)) => {
			let a = arg_parse!(command_args::parse_receive_args(&args));
			command::receive(inst_wallet(), &global_wallet_args, a)
		}
		("finalize", Some(args)) => {
			let a = arg_parse!(command_args::parse_finalize_args(&args));
			command::finalize(inst_wallet(), a)
		}
		("info", Some(args)) => {
			let a = arg_parse!(command_args::parse_info_args(&args));
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
			let a = arg_parse!(command_args::parse_txs_args(&args));
			command::txs(
				inst_wallet(),
				&global_wallet_args,
				a,
				wallet_config.dark_background_color_scheme.unwrap_or(true),
			)
		}
		("repost", Some(args)) => {
			let a = arg_parse!(command_args::parse_repost_args(&args));
			command::repost(inst_wallet(), a)
		}
		("cancel", Some(args)) => {
			let a = arg_parse!(command_args::parse_cancel_args(&args));
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
		println!(
			"Command '{}' completed successfully",
			wallet_args.subcommand().0
		);
		0
	}
}
