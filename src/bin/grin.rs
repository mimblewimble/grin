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
extern crate slog;
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

use clap::{App, Arg, SubCommand};

use config::config::{SERVER_CONFIG_FILE_NAME, WALLET_CONFIG_FILE_NAME};
use core::global;
use util::{init_logger, LOGGER};

// include build information
pub mod built_info {
	include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub fn info_strings() -> (String, String, String) {
	(
		format!(
			"This is Grin version {}{}, built for {} by {}.",
			built_info::PKG_VERSION,
			built_info::GIT_VERSION.map_or_else(|| "".to_owned(), |v| format!(" (git {})", v)),
			built_info::TARGET,
			built_info::RUSTC_VERSION
		).to_string(),
		format!(
			"Built with profile \"{}\", features \"{}\" on {}.",
			built_info::PROFILE,
			built_info::FEATURES_STR,
			built_info::BUILT_TIME_UTC
		).to_string(),
		format!("Dependencies:\n {}", built_info::DEPENDENCIES_STR).to_string(),
	)
}

fn log_build_info() {
	let (basic_info, detailed_info, deps) = info_strings();
	info!(LOGGER, "{}", basic_info);
	debug!(LOGGER, "{}", detailed_info);
	trace!(LOGGER, "{}", deps);
}

fn main() {
	let args = App::new("Grin")
		.version(crate_version!())
		.author("The Grin Team")
		.about("Lightweight implementation of the MimbleWimble protocol.")
    // specification of all the server commands and options
    .subcommand(SubCommand::with_name("server")
                .about("Control the Grin server")
                .arg(Arg::with_name("config_file")
                     .short("c")
                     .long("config_file")
                     .help("Path to a grin-server.toml configuration file")
                     .takes_value(true))
                .arg(Arg::with_name("port")
                     .short("p")
                     .long("port")
                     .help("Port to start the P2P server on")
                     .takes_value(true))
                .arg(Arg::with_name("api_port")
                     .short("a")
                     .long("api_port")
                     .help("Port on which to start the api server (e.g. transaction pool api)")
                     .takes_value(true))
                .arg(Arg::with_name("seed")
                     .short("s")
                     .long("seed")
                     .help("Override seed node(s) to connect to")
                     .takes_value(true))
                .arg(Arg::with_name("wallet_url")
                     .short("w")
                     .long("wallet_url")
                     .help("The wallet listener to which mining rewards will be sent")
                     .takes_value(true))
                .subcommand(SubCommand::with_name("config")
                            .about("Generate a configuration grin-server.toml file in the current directory"))
                .subcommand(SubCommand::with_name("start")
                            .about("Start the Grin server as a daemon"))
                .subcommand(SubCommand::with_name("stop")
                            .about("Stop the Grin server daemon"))
                .subcommand(SubCommand::with_name("run")
                            .about("Run the Grin server in this console")))

    // specification of all the client commands and options
    .subcommand(SubCommand::with_name("client")
                .about("Communicates with the Grin server")
                .subcommand(SubCommand::with_name("status")
                            .about("Current status of the Grin chain"))
				.subcommand(SubCommand::with_name("listconnectedpeers")
							.about("Print a list of currently connected peers"))
				.subcommand(SubCommand::with_name("ban")
							.about("Ban peer")
							.arg(Arg::with_name("peer")
								.short("p")
								.long("peer")
								.help("Peer ip and port (e.g. 10.12.12.13:13414)")
								.required(true)
								.takes_value(true)))
				.subcommand(SubCommand::with_name("unban")
							.about("Unban peer")
							.arg(Arg::with_name("peer")
								.short("p")
								.long("peer")
								.help("Peer ip and port (e.g. 10.12.12.13:13414)")
								.required(true)
								.takes_value(true))))


	// specification of the wallet commands and options
	.subcommand(SubCommand::with_name("wallet")
		.about("Wallet software for Grin")
		.arg(Arg::with_name("pass")
			.short("p")
			.long("pass")
			.help("Wallet passphrase used to generate the private key seed")
			.takes_value(true)
			.default_value(""))
		.arg(Arg::with_name("data_dir")
			.short("dd")
			.long("data_dir")
			.help("Directory in which to store wallet files (defaults to current \
			directory)")
			.takes_value(true))
		.arg(Arg::with_name("external")
			.short("e")
			.long("external")
			.help("Listen on 0.0.0.0 interface to allow external connections (default is 127.0.0.1)")
			.takes_value(false))
		.arg(Arg::with_name("show_spent")
			.short("s")
			.long("show_spent")
			.help("Show spent outputs on wallet output command")
			.takes_value(false))
		.arg(Arg::with_name("api_server_address")
			.short("a")
			.long("api_server_address")
			.help("Api address of running node on which to check inputs and post transactions")
			.takes_value(true))

		.subcommand(SubCommand::with_name("listen")
			.about("Runs the wallet in listening mode waiting for transactions.")
			.arg(Arg::with_name("port")
				.short("l")
				.long("port")
				.help("Port on which to run the wallet listener")
				.takes_value(true)))

		.subcommand(SubCommand::with_name("owner_api")
			.about("Runs the wallet's local web API."))

		.subcommand(SubCommand::with_name("web")
			.about("Runs the local web wallet which can be accessed through a browser"))

		.subcommand(SubCommand::with_name("send")
			.about("Builds a transaction to send coins and sends it to the specified \
			 listener directly.")
			.arg(Arg::with_name("amount")
				.help("Number of coins to send with optional fraction, e.g. 12.423")
				.index(1))
			.arg(Arg::with_name("minimum_confirmations")
				.help("Minimum number of confirmations required for an output to be spendable.")
				.short("c")
				.long("min_conf")
				.default_value("1")
				.takes_value(true))
			.arg(Arg::with_name("selection_strategy")
				.help("Coin/Output selection strategy.")
				.short("s")
				.long("selection")
				.possible_values(&["all", "smallest"])
				.default_value("all")
				.takes_value(true))
			.arg(Arg::with_name("change_outputs")
				.help("Number of change outputs to generate (mainly for testing).")
				.short("o")
				.long("change_outputs")
				.default_value("1")
				.takes_value(true))
			.arg(Arg::with_name("method")
				.help("Method for sending this transaction.")
				.short("m")
				.long("method")
				.possible_values(&["http", "file"])
				.default_value("http")
				.takes_value(true))
			.arg(Arg::with_name("dest")
				.help("Send the transaction to the provided server (start with http://) or save as file.")
				.short("d")
				.long("dest")
				.takes_value(true))
			.arg(Arg::with_name("fluff")
				.help("Fluff the transaction (ignore Dandelion relay protocol)")
				.short("f")
				.long("fluff")))
			.arg(Arg::with_name("stored_tx")
				.help("If present, use the previously stored Unconfirmed transaction with given id.")
				.short("t")
				.long("stored_tx")
				.takes_value(true))

		.subcommand(SubCommand::with_name("receive")
			.about("Processes a transaction file to accept a transfer from a sender.")
			.arg(Arg::with_name("input")
				.help("Partial transaction to process, expects the sender's transaction file.")
				.short("i")
				.long("input")
				.takes_value(true)))

		.subcommand(SubCommand::with_name("finalize")
			.about("Processes a receiver's transaction file to finalize a transfer.")
			.arg(Arg::with_name("input")
				.help("Partial transaction to process, expects the receiver's transaction file.")
				.short("i")
				.long("input")
				.takes_value(true))
			.arg(Arg::with_name("fluff")
				.help("Fluff the transaction (ignore Dandelion relay protocol)")
				.short("f")
				.long("fluff")))

		.subcommand(SubCommand::with_name("burn")
			.about("** TESTING ONLY ** Burns the provided amount to a known \
				key. Similar to send but burns an output to allow single-party \
				transactions.")
			.arg(Arg::with_name("amount")
				.help("Number of coins to burn")
				.index(1))
			.arg(Arg::with_name("minimum_confirmations")
				.help("Minimum number of confirmations required for an output to be spendable.")
				.short("c")
				.long("min_conf")
				.default_value("1")
				.takes_value(true)))

		.subcommand(SubCommand::with_name("outputs")
			.about("raw wallet output info (list of outputs)"))

		.subcommand(SubCommand::with_name("txs")
			.about("Display transaction information")
			.arg(Arg::with_name("id")
				.help("If specified, display transaction with given ID and all associated Inputs/Outputs")
				.short("i")
				.long("id")
				.takes_value(true)))

		.subcommand(SubCommand::with_name("repost")
			.about("Reposts a stored, completed but unconfirmed transaction to the chain, or dumps it to a file")
			.arg(Arg::with_name("id")
				.help("Transaction ID Containing the stored completed transaction")
				.short("i")
				.long("id")
				.takes_value(true))
			.arg(Arg::with_name("dumpfile")
				.help("File name to duMp the tranaction to instead of posting")
				.short("m")
				.long("dumpfile")
				.takes_value(true))
			.arg(Arg::with_name("fluff")
				.help("Fluff the transaction (ignore Dandelion relay protocol)")
				.short("f")
				.long("fluff")))

		.subcommand(SubCommand::with_name("cancel")
			.about("Cancels an previously created transaction, freeing previously locked outputs for use again")
			.arg(Arg::with_name("id")
				.help("The ID of the transaction to cancel")
				.short("i")
				.long("id")
				.takes_value(true)))

		.subcommand(SubCommand::with_name("info")
			.about("basic wallet contents summary"))

		.subcommand(SubCommand::with_name("init")
			.about("Initialize a new wallet seed file and database.")
			.arg(Arg::with_name("here")
				.short("h")
				.long("here")
				.help("Create wallet files in the current directory instead of the default ~/.grin directory")
				.takes_value(false)))

		.subcommand(SubCommand::with_name("restore")
			.about("Attempt to restore wallet contents from the chain using seed and password. \
				NOTE: Backup wallet.* and run `wallet listen` before running restore.")))

	.get_matches();
	let mut wallet_config = None;
	let mut node_config = None;

	// Deal with configuration file creation
	match args.subcommand() {
		("server", Some(server_args)) => {
			// If it's just a server config command, do it and exit
			if let ("config", Some(_)) = server_args.subcommand() {
				cmd::config_command_server(SERVER_CONFIG_FILE_NAME);
				return;
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
				if let ("init", Some(_)) = wallet_args.subcommand() {
				} else {
					println!("Wallet seed file doesn't exist. Run `grin wallet -p [password] init` first");
					exit(1);
				}
			}
			let mut l = w.members.as_mut().unwrap().logging.clone().unwrap();
			l.tui_running = Some(false);
			init_logger(Some(l));
			warn!(
				LOGGER,
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
					LOGGER,
					"Using configuration file at {}",
					file_path.to_str().unwrap()
				);
			} else {
				info!(LOGGER, "Node configuration file not found, using default");
			}
			node_config = Some(s);
		}
	}

	log_build_info();

	match args.subcommand() {
		// server commands and options
		("server", Some(server_args)) => {
			cmd::server_command(Some(server_args), node_config.unwrap());
		}

		// client commands and options
		("client", Some(client_args)) => {
			cmd::client_command(client_args, node_config.unwrap());
		}

		// client commands and options
		("wallet", Some(wallet_args)) => {
			cmd::wallet_command(wallet_args, wallet_config.unwrap());
		}

		// If nothing is specified, try to just use the config file instead
		// this could possibly become the way to configure most things
		// with most command line options being phased out
		_ => {
			cmd::server_command(None, node_config.unwrap());
		}
	}
}
