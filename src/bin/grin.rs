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

extern crate blake2_rfc as blake2;
extern crate chrono;
#[macro_use]
extern crate clap;
extern crate ctrlc;
extern crate cursive;
extern crate daemonize;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate log;
extern crate failure;
extern crate term;

extern crate grin_api as api;
extern crate grin_config as config;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_servers as servers;
extern crate grin_util as util;
extern crate grin_wallet;

mod cmd;
pub mod tui;

use std::process::exit;

use clap::App;

use config::config::{SERVER_CONFIG_FILE_NAME, WALLET_CONFIG_FILE_NAME};
use core::global;
use util::init_logger;

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
		).to_string(),
		format!(
			"Built with profile \"{}\", features \"{}\".",
			built_info::PROFILE,
			built_info::FEATURES_STR,
		).to_string(),
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

	// Deal with configuration file creation
	match args.subcommand() {
		("server", Some(server_args)) => {
			// If it's just a server config command, do it and exit
			if let ("config", Some(_)) = server_args.subcommand() {
				cmd::config_command_server(SERVER_CONFIG_FILE_NAME);
				return 0;
			}
		}
		("wallet", Some(wallet_args)) => {
			// wallet init command should spit out its config file then continue
			// (if desired)
			if let ("init", Some(init_args)) = wallet_args.subcommand() {
				if init_args.is_present("here") {
					cmd::config_command_wallet(WALLET_CONFIG_FILE_NAME);
				}
			}
		}
		_ => {}
	}

	match args.subcommand() {
		// If it's a wallet command, try and load a wallet config file
		("wallet", Some(wallet_args)) => {
			let mut w = config::initial_setup_wallet().unwrap_or_else(|e| {
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
		// Otherwise load up the node config as usual
		_ => {
			let mut s = config::initial_setup_server().unwrap_or_else(|e| {
				panic!("Error loading server configuration: {}", e);
			});
			let mut l = s.members.as_mut().unwrap().logging.clone().unwrap();
			let run_tui = s.members.as_mut().unwrap().server.run_tui;
			if let Some(true) = run_tui {
				l.log_to_stdout = false;
				l.tui_running = Some(true);
			}
			init_logger(Some(l));
			global::set_mining_mode(s.members.as_mut().unwrap().server.clone().chain_type);
			if let Some(file_path) = &s.config_file_path {
				info!(
					"Using configuration file at {}",
					file_path.to_str().unwrap()
				);
			} else {
				info!("Node configuration file not found, using default");
			}
			node_config = Some(s);
		}
	}

	log_build_info();

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
