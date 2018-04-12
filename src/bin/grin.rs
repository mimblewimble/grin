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
#[macro_use]
extern crate clap;
extern crate cursive;
extern crate daemonize;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate time;

extern crate grin_api as api;
extern crate grin_config as config;
extern crate grin_core as core;
extern crate grin_servers as servers;
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

mod client;
pub mod tui;

use std::thread;
use std::sync::Arc;
use std::time::Duration;
use std::env::current_dir;
use std::process::exit;

use clap::{App, Arg, ArgMatches, SubCommand};
use daemonize::Daemonize;

use config::GlobalConfig;
use core::global;
use core::core::amount_to_hr_string;
use util::{init_logger, LoggingConfig, LOGGER};
use tui::ui;

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

/// wrap below to allow UI to clean up on stop
fn start_server(config: servers::ServerConfig) {
	start_server_tui(config);
	// Just kill process for now, otherwise the process
	// hangs around until sigint because the API server
	// currently has no shutdown facility
	println!("Shutting down...");
	thread::sleep(Duration::from_millis(1000));
	println!("Shutdown complete.");
	exit(0);
}

fn start_server_tui(config: servers::ServerConfig) {
	// Run the UI controller.. here for now for simplicity to access
	// everything it might need
	if config.run_tui.is_some() && config.run_tui.unwrap() {
		println!("Starting GRIN in UI mode...");
		servers::Server::start(config, |serv: Arc<servers::Server>| {
			let _ = thread::Builder::new()
				.name("ui".to_string())
				.spawn(move || {
					let mut controller = ui::Controller::new().unwrap_or_else(|e| {
						panic!("Error loading UI controller: {}", e);
					});
					controller.run(serv.clone());
				});
		}).unwrap();
	} else {
		servers::Server::start(config, |_| {}).unwrap();
	}
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
                .arg(Arg::with_name("mine")
                     .short("m")
                     .long("mine")
                     .help("Starts the debugging mining loop"))
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
								.takes_value(true)))
				.subcommand(SubCommand::with_name("unban")
							.about("Unban peer")
							.arg(Arg::with_name("peer")
								.short("p")
								.long("peer")
								.help("Peer ip and port (e.g. 10.12.12.13:13414)")
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
			.about("raw wallet info (list of outputs)"))

		.subcommand(SubCommand::with_name("info")
			.about("basic wallet contents summary"))

		.subcommand(SubCommand::with_name("init")
			.about("Initialize a new wallet seed file."))

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

	if global_config.using_config_file {
		// initialise the logger
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
	} else {
		init_logger(Some(LoggingConfig::default()));
	}

	log_build_info();

	match args.subcommand() {
		// server commands and options
		("server", Some(server_args)) => {
			server_command(Some(server_args), global_config);
		}

		// client commands and options
		("client", Some(client_args)) => {
			client_command(client_args, global_config);
		}

		// client commands and options
		("wallet", Some(wallet_args)) => {
			wallet_command(wallet_args, global_config);
		}

		// If nothing is specified, try to just use the config file instead
		// this could possibly become the way to configure most things
		// with most command line options being phased out
		_ => {
			if global_config.using_config_file {
				server_command(None, global_config);
			} else {
				// won't attempt to just start with defaults,
				// and will reject
				println!("Unknown command, and no configuration file was found.");
				println!("Use 'grin help' for a list of all commands.");
			}
		}
	}
}

/// Handles the server part of the command line, mostly running, starting and
/// stopping the Grin blockchain server. Processes all the command line
/// arguments
/// to build a proper configuration and runs Grin with that configuration.
fn server_command(server_args: Option<&ArgMatches>, mut global_config: GlobalConfig) {
	if global_config.using_config_file {
		info!(
			LOGGER,
			"Starting the Grin server from configuration file at {}",
			global_config.config_file_path.unwrap().to_str().unwrap()
		);
		global::set_mining_mode(
			global_config
				.members
				.as_mut()
				.unwrap()
				.server
				.clone()
				.chain_type,
		);
	} else {
		info!(
			LOGGER,
			"Starting the Grin server (no configuration file) ..."
		);
	}

	// just get defaults from the global config
	let mut server_config = global_config.members.as_ref().unwrap().server.clone();

	if let Some(a) = server_args {
		if let Some(port) = a.value_of("port") {
			server_config.p2p_config.port = port.parse().unwrap();
		}

		if let Some(api_port) = a.value_of("api_port") {
			let default_ip = "0.0.0.0";
			server_config.api_http_addr = format!("{}:{}", default_ip, api_port);
		}

		if a.is_present("mine") {
			server_config.mining_config.as_mut().unwrap().enable_mining = true;
		}

		if let Some(wallet_url) = a.value_of("wallet_url") {
			server_config
				.mining_config
				.as_mut()
				.unwrap()
				.wallet_listener_url = wallet_url.to_string();
		}

		if let Some(seeds) = a.values_of("seed") {
			server_config.seeding_type = servers::Seeding::List;
			server_config.seeds = Some(seeds.map(|s| s.to_string()).collect());
		}
	}

	if let Some(true) = server_config.run_wallet_listener {
		let mut wallet_config = global_config.members.unwrap().wallet;
		let wallet_seed = match wallet::WalletSeed::from_file(&wallet_config) {
			Ok(ws) => ws,
			Err(_) => wallet::WalletSeed::init_file(&wallet_config)
				.expect("Failed to create wallet seed file."),
		};
		let mut keychain = wallet_seed
			.derive_keychain("")
			.expect("Failed to derive keychain from seed file and passphrase.");

		let _ = thread::Builder::new()
			.name("wallet_listener".to_string())
			.spawn(move || {
				wallet::server::start_rest_apis(wallet_config, keychain);
			});
	}

	// start the server in the different run modes (interactive or daemon)
	if let Some(a) = server_args {
		match a.subcommand() {
			("run", _) => {
				start_server(server_config);
			}
			("start", _) => {
				let daemonize = Daemonize::new()
					.pid_file("/tmp/grin.pid")
					.chown_pid_file(true)
					.working_directory(current_dir().unwrap())
					.privileged_action(move || {
						start_server(server_config.clone());
						loop {
							thread::sleep(Duration::from_secs(60));
						}
					});
				match daemonize.start() {
					Ok(_) => info!(LOGGER, "Grin server successfully started."),
					Err(e) => error!(LOGGER, "Error starting: {}", e),
				}
			}
			("stop", _) => println!("TODO. Just 'kill $pid' for now. Maybe /tmp/grin.pid is $pid"),
			(cmd, _) => {
				println!(":: {:?}", server_args);
				panic!(
					"Unknown server command '{}', use 'grin help server' for details",
					cmd
				);
			}
		}
	} else {
		start_server(server_config);
	}
}

fn client_command(client_args: &ArgMatches, global_config: GlobalConfig) {
	// just get defaults from the global config
	let server_config = global_config.members.unwrap().server;

	match client_args.subcommand() {
		("status", Some(_)) => {
			client::show_status(&server_config);
		}
		("listconnectedpeers", Some(_)) => {
			client::list_connected_peers(&server_config);
		}
		("ban", Some(peer_args)) => {
			if let Some(peer) = peer_args.value_of("peer") {
				if let Ok(addr) = peer.parse() {
					client::ban_peer(&server_config, &addr);
				} else {
					panic!("Invalid peer address format");
				}
			}
		}
		("unban", Some(peer_args)) => {
			if let Some(peer) = peer_args.value_of("peer") {
				if let Ok(addr) = peer.parse() {
					client::unban_peer(&server_config, &addr);
				} else {
					panic!("Invalid peer address format");
				}
			}
		}
		_ => panic!("Unknown client command, use 'grin help client' for details"),
	}
}

fn wallet_command(wallet_args: &ArgMatches, global_config: GlobalConfig) {
	// just get defaults from the global config
	let mut wallet_config = global_config.members.unwrap().wallet;

	if wallet_args.is_present("external") {
		wallet_config.api_listen_interface = "0.0.0.0".to_string();
	}

	if let Some(dir) = wallet_args.value_of("dir") {
		wallet_config.data_file_dir = dir.to_string().clone();
	}

	if let Some(sa) = wallet_args.value_of("api_server_address") {
		wallet_config.check_node_api_http_addr = sa.to_string().clone();
	}

	let mut show_spent = false;
	if wallet_args.is_present("show_spent") {
		show_spent = true;
	}

	// Derive the keychain based on seed from seed file and specified passphrase.
	// Generate the initial wallet seed if we are running "wallet init".
	if let ("init", Some(_)) = wallet_args.subcommand() {
		wallet::WalletSeed::init_file(&wallet_config).expect("Failed to init wallet seed file.");

		// we are done here with creating the wallet, so just return
		return;
	}

	let wallet_seed =
		wallet::WalletSeed::from_file(&wallet_config).expect("Failed to read wallet seed file.");
	let passphrase = wallet_args
		.value_of("pass")
		.expect("Failed to read passphrase.");
	let mut keychain = wallet_seed
		.derive_keychain(&passphrase)
		.expect("Failed to derive keychain from seed file and passphrase.");

	match wallet_args.subcommand() {
		("listen", Some(listen_args)) => {
			if let Some(port) = listen_args.value_of("port") {
				wallet_config.api_listen_port = port.parse().unwrap();
			}
			wallet::server::start_rest_apis(wallet_config, keychain);
		}
		("send", Some(send_args)) => {
			let amount = send_args
				.value_of("amount")
				.expect("Amount to send required");
			let amount = core::core::amount_from_hr_string(amount)
				.expect("Could not parse amount as a number with optional decimal point.");
			let minimum_confirmations: u64 = send_args
				.value_of("minimum_confirmations")
				.unwrap()
				.parse()
				.expect("Could not parse minimum_confirmations as a whole number.");
			let selection_strategy = send_args
				.value_of("selection_strategy")
				.expect("Selection strategy required");
			let dest = send_args
				.value_of("dest")
				.expect("Destination wallet address required");
			let mut fluff = false;
			if send_args.is_present("fluff") {
				fluff = true;
			}
			let max_outputs = 500;
			let result = wallet::issue_send_tx(
				&wallet_config,
				&mut keychain,
				amount,
				minimum_confirmations,
				dest.to_string(),
				max_outputs,
				selection_strategy == "all",
				fluff,
			);
			match result {
				Ok(_) => info!(
					LOGGER,
					"Tx sent: {} grin to {} (strategy '{}')",
					amount_to_hr_string(amount),
					dest,
					selection_strategy,
				),
				Err(e) => match e.kind() {
					wallet::ErrorKind::NotEnoughFunds(available) => {
						error!(
							LOGGER,
							"Tx not sent: insufficient funds (max: {})",
							amount_to_hr_string(available),
						);
					}
					wallet::ErrorKind::FeeExceedsAmount {
						sender_amount,
						recipient_fee,
					} => {
						error!(
								LOGGER,
								"Recipient rejected the transfer because transaction fee ({}) exceeded amount ({}).",
								amount_to_hr_string(recipient_fee),
								amount_to_hr_string(sender_amount)
							);
					}
					_ => {
						error!(LOGGER, "Tx not sent: {:?}", e);
					}
				},
			};
		}
		("burn", Some(send_args)) => {
			let amount = send_args
				.value_of("amount")
				.expect("Amount to burn required");
			let amount = core::core::amount_from_hr_string(amount)
				.expect("Could not parse amount as number with optional decimal point.");
			let minimum_confirmations: u64 = send_args
				.value_of("minimum_confirmations")
				.unwrap()
				.parse()
				.expect("Could not parse minimum_confirmations as a whole number.");
			let max_outputs = 500;
			wallet::issue_burn_tx(
				&wallet_config,
				&keychain,
				amount,
				minimum_confirmations,
				max_outputs,
			).unwrap();
		}
		("info", Some(_)) => {
			wallet::show_info(&wallet_config, &keychain);
		}
		("outputs", Some(_)) => {
			wallet::show_outputs(&wallet_config, &keychain, show_spent);
		}
		("restore", Some(_)) => {
			let _ = wallet::restore(&wallet_config, &keychain);
		}
		_ => panic!("Unknown wallet command, use 'grin help wallet' for details"),
	}
}
