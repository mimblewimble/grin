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

use clap::{App, Arg, SubCommand};

use config::GlobalConfig;
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

		.subcommand(SubCommand::with_name("receive")
			.about("Processes a JSON transaction file.")
			.arg(Arg::with_name("input")
				.help("Partial transaction to process, expects a JSON file.")
				.short("i")
				.long("input")
				.takes_value(true)))

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
			.arg(Arg::with_name("dest")
				.help("Send the transaction to the provided server")
				.short("d")
				.long("dest")
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
			.about("Initialize a new wallet seed file and database."))

		.subcommand(SubCommand::with_name("restore")
			.about("Attempt to restore wallet contents from the chain using seed and password. \
				NOTE: Backup wallet.* and run `wallet listen` before running restore.")))

	.get_matches();

	// load a global config object,
	// then modify that object with any switches
	// found so that the switches override the
	// global config file

	// This will return a global config object,
	// which will either contain defaults for all // of the config structures or a
	// configuration
	// read from a config file

	let mut global_config = GlobalConfig::new(None).unwrap_or_else(|e| {
		panic!("Error parsing config file: {}", e);
	});

	// initialize the logger
	let mut log_conf = global_config
		.members
		.as_mut()
		.unwrap()
		.logging
		.clone()
		.unwrap();
	let run_tui = global_config.members.as_mut().unwrap().server.run_tui;
	if run_tui.is_some() && run_tui.unwrap() && args.subcommand().0 != "wallet" {
		log_conf.log_to_stdout = false;
		log_conf.tui_running = Some(true);
	}
	init_logger(Some(log_conf));
	global::set_mining_mode(
		global_config
			.members
			.as_mut()
			.unwrap()
			.server
			.clone()
			.chain_type,
	);

	log_build_info();

	if let Some(file_path) = &global_config.config_file_path {
		info!(
			LOGGER,
			"Found configuration file at {}",
			file_path.to_str().unwrap()
		);
	} else {
		info!(LOGGER, "configuration file not found, using default");
	}

	match args.subcommand() {
		// server commands and options
		("server", Some(server_args)) => {
			cmd::server_command(Some(server_args), global_config);
		}

		// client commands and options
		("client", Some(client_args)) => {
			cmd::client_command(client_args, global_config);
		}

		// client commands and options
		("wallet", Some(wallet_args)) => {
			cmd::wallet_command(wallet_args, global_config);
		}

		// If nothing is specified, try to just use the config file instead
		// this could possibly become the way to configure most things
		// with most command line options being phased out
		_ => {
			cmd::server_command(None, global_config);
		}
	}
}


