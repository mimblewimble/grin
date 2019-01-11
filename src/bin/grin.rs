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

//! Main for building the binary of a Grin peer-to-peer node.

#[macro_use]
extern crate clap;
use ctrlc;

use serde_json;
#[macro_use]
extern crate log;
use crate::config::config::{SERVER_CONFIG_FILE_NAME, WALLET_CONFIG_FILE_NAME};
use crate::core::global;
use crate::util::init_logger;
use clap::App;
use grin_api as api;
use grin_config as config;
use grin_core as core;
use grin_p2p as p2p;
use grin_servers as servers;
use grin_util as util;
use grin_wallet;
use std::process::exit;
use term;

mod cmd;
pub mod tui;

// include build information
pub mod built_info {
	include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub fn info_strings() -> (String, String) {
	(
		format!(
			"This is Grin version {}{}, built for {} by {}.",
			built_info::PKG_VERSION,
			built_info::GIT_VERSION.map_or_else(|| "".to_owned(), |v| format!(" (git {})", v)),
			built_info::TARGET,
			built_info::RUSTC_VERSION,
		)
		.to_string(),
		format!(
			"Built with profile \"{}\", features \"{}\".",
			built_info::PROFILE,
			built_info::FEATURES_STR,
		)
		.to_string(),
	)
}

fn log_build_info() {
	let (basic_info, detailed_info) = info_strings();
	info!("{}", basic_info);
	debug!("{}", detailed_info);
}

fn main() {
	let exit_code = real_main();
	std::process::exit(exit_code);
}

fn real_main() -> i32 {
	let yml = load_yaml!("grin.yml");
	let args = App::from_yaml(yml).get_matches();
	let mut wallet_config = None;
	let mut node_config = None;

	let chain_type = if args.is_present("floonet") {
		global::ChainTypes::Floonet
	} else if args.is_present("usernet") {
		global::ChainTypes::UserTesting
	} else {
		global::ChainTypes::Mainnet
	};

	// TODO remove for mainnet
	if chain_type == global::ChainTypes::Mainnet {
		println!("Mainnet not ready yet! In the meantime run 'grin --floonet ...'");
		exit(1);
	}

	// Deal with configuration file creation
	match args.subcommand() {
		("server", Some(server_args)) => {
			// If it's just a server config command, do it and exit
			if let ("config", Some(_)) = server_args.subcommand() {
				cmd::config_command_server(&chain_type, SERVER_CONFIG_FILE_NAME);
				return 0;
			}
		}
		("wallet", Some(wallet_args)) => {
			// wallet init command should spit out its config file then continue
			// (if desired)
			if let ("init", Some(init_args)) = wallet_args.subcommand() {
				if init_args.is_present("here") {
					cmd::config_command_wallet(&chain_type, WALLET_CONFIG_FILE_NAME);
				}
			}
		}
		_ => {}
	}

	// Load relevant config
	match args.subcommand() {
		// If it's a wallet command, try and load a wallet config file
		("wallet", Some(wallet_args)) => {
			let mut w = config::initial_setup_wallet(&chain_type).unwrap_or_else(|e| {
				panic!("Error loading wallet configuration: {}", e);
			});
			if !cmd::seed_exists(w.members.as_ref().unwrap().wallet.clone()) {
				if "init" == wallet_args.subcommand().0 || "recover" == wallet_args.subcommand().0 {
				} else {
					println!("Wallet seed file doesn't exist. Run `grin wallet init` first");
					exit(1);
				}
			}
			let mut l = w.members.as_mut().unwrap().logging.clone().unwrap();
			l.tui_running = Some(false);
			init_logger(Some(l));
			info!(
				"Using wallet configuration file at {}",
				w.config_file_path.as_ref().unwrap().to_str().unwrap()
			);
			wallet_config = Some(w);
		}
		// When the subscommand is 'server' take into account the 'config_file' flag
		("server", Some(server_args)) => {
			if let Some(_path) = server_args.value_of("config_file") {
				node_config = Some(config::GlobalConfig::new(_path).unwrap_or_else(|e| {
					panic!("Error loading server configuration: {}", e);
				}));
			} else {
				node_config = Some(
					config::initial_setup_server(&chain_type).unwrap_or_else(|e| {
						panic!("Error loading server configuration: {}", e);
					}),
				);
			}
		}
		// Otherwise load up the node config as usual
		_ => {
			node_config = Some(
				config::initial_setup_server(&chain_type).unwrap_or_else(|e| {
					panic!("Error loading server configuration: {}", e);
				}),
			);
		}
	}

	if let Some(mut config) = node_config.clone() {
		let mut l = config.members.as_mut().unwrap().logging.clone().unwrap();
		let run_tui = config.members.as_mut().unwrap().server.run_tui;
		if let Some(true) = run_tui {
			l.log_to_stdout = false;
			l.tui_running = Some(true);
		}
		init_logger(Some(l));

		global::set_mining_mode(config.members.unwrap().server.clone().chain_type);

		if let Some(file_path) = &config.config_file_path {
			info!(
				"Using configuration file at {}",
				file_path.to_str().unwrap()
			);
		} else {
			info!("Node configuration file not found, using default");
		}
	}

	log_build_info();

	// Execute subcommand
	match args.subcommand() {
		// server commands and options
		("server", Some(server_args)) => {
			cmd::server_command(Some(server_args), node_config.unwrap())
		}

		// client commands and options
		("client", Some(client_args)) => cmd::client_command(client_args, node_config.unwrap()),

		// client commands and options
		("wallet", Some(wallet_args)) => cmd::wallet_command(wallet_args, wallet_config.unwrap()),

		// If nothing is specified, try to just use the config file instead
		// this could possibly become the way to configure most things
		// with most command line options being phased out
		_ => cmd::server_command(None, node_config.unwrap()),
	}
}
