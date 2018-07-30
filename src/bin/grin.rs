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
extern crate ctrlc;
extern crate cursive;
extern crate daemonize;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate chrono;

extern crate grin_api as api;
extern crate grin_config as config;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_servers as servers;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

mod client;
pub mod tui;

use std::env::current_dir;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clap::{App, Arg, ArgMatches, SubCommand};
use daemonize::Daemonize;

use config::GlobalConfig;
use core::core::amount_to_hr_string;
use core::global;
use tui::ui;
use util::{init_logger, LOGGER};
use wallet::{libwallet, HTTPWalletClient, LMDBBackend, WalletConfig, WalletInst};

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
			let running = Arc::new(AtomicBool::new(true));
			let r = running.clone();
			let _ = thread::Builder::new()
				.name("ui".to_string())
				.spawn(move || {
					let mut controller = ui::Controller::new().unwrap_or_else(|e| {
						panic!("Error loading UI controller: {}", e);
					});
					controller.run(serv.clone(), r);
				});
			ctrlc::set_handler(move || {
				running.store(false, Ordering::SeqCst);
			}).expect("Error setting Ctrl-C handler");
		}).unwrap();
	} else {
		servers::Server::start(config, |serv: Arc<servers::Server>| {
			let running = Arc::new(AtomicBool::new(true));
			let r = running.clone();
			ctrlc::set_handler(move || {
				r.store(false, Ordering::SeqCst);
			}).expect("Error setting Ctrl-C handler");
			while running.load(Ordering::SeqCst) {
				thread::sleep(Duration::from_secs(1));
			}
			warn!(LOGGER, "Received SIGINT (Ctrl+C).");
			serv.stop();
		}).unwrap();
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

	if let Some(file_path) = &global_config.config_file_path {
		info!(
			LOGGER,
			"Found configuration file at {}",
			file_path.to_str().unwrap()
		);
	} else {
		info!(LOGGER, "configuration file not found, using default");
	}

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
			server_command(None, global_config);
		}
	}
}

/// Handles the server part of the command line, mostly running, starting and
/// stopping the Grin blockchain server. Processes all the command line
/// arguments
/// to build a proper configuration and runs Grin with that configuration.
fn server_command(server_args: Option<&ArgMatches>, mut global_config: GlobalConfig) {
	global::set_mining_mode(
		global_config
			.members
			.as_mut()
			.unwrap()
			.server
			.clone()
			.chain_type,
	);

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

		if let Some(wallet_url) = a.value_of("wallet_url") {
			server_config
				.stratum_mining_config
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
		let mut wallet_config = global_config.members.as_ref().unwrap().wallet.clone();
		init_wallet_seed(wallet_config.clone());
		let wallet = instantiate_wallet(wallet_config.clone(), "");

		let _ = thread::Builder::new()
			.name("wallet_listener".to_string())
			.spawn(move || {
				wallet::controller::foreign_listener(wallet, &wallet_config.api_listen_addr())
					.unwrap_or_else(|e| {
						panic!(
							"Error creating wallet listener: {:?} Config: {:?}",
							e, wallet_config
						)
					});
			});
	}
	if let Some(true) = server_config.run_wallet_owner_api {
		let mut wallet_config = global_config.members.unwrap().wallet;
		let wallet = instantiate_wallet(wallet_config.clone(), "");
		init_wallet_seed(wallet_config.clone());

		let _ = thread::Builder::new()
			.name("wallet_owner_listener".to_string())
			.spawn(move || {
				wallet::controller::owner_listener(wallet, "127.0.0.1:13420").unwrap_or_else(|e| {
					panic!(
						"Error creating wallet api listener: {:?} Config: {:?}",
						e, wallet_config
					)
				});
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
			let peer = peer_args.value_of("peer").unwrap();

			if let Ok(addr) = peer.parse() {
				client::ban_peer(&server_config, &addr);
			} else {
				panic!("Invalid peer address format");
			}
		}
		("unban", Some(peer_args)) => {
			let peer = peer_args.value_of("peer").unwrap();

			if let Ok(addr) = peer.parse() {
				client::unban_peer(&server_config, &addr);
			} else {
				panic!("Invalid peer address format");
			}
		}
		_ => panic!("Unknown client command, use 'grin help client' for details"),
	}
}

fn init_wallet_seed(wallet_config: WalletConfig) {
	if let Err(_) = wallet::WalletSeed::from_file(&wallet_config) {
		wallet::WalletSeed::init_file(&wallet_config).expect("Failed to create wallet seed file.");
	};
}

fn instantiate_wallet(
	wallet_config: WalletConfig,
	passphrase: &str,
) -> Box<WalletInst<HTTPWalletClient, keychain::ExtKeychain>> {
	if wallet::needs_migrate(&wallet_config.data_file_dir) {
		// Migrate wallet automatically
		warn!(LOGGER, "Migrating legacy File-Based wallet to LMDB Format");
		if let Err(e) = wallet::migrate(&wallet_config.data_file_dir, passphrase) {
			error!(LOGGER, "Error while trying to migrate wallet: {:?}", e);
			error!(LOGGER, "Please ensure your file wallet files exist and are not corrupted, and that your password is correct");
			panic!();
		} else {
			warn!(LOGGER, "Migration successful. Using LMDB Wallet backend");
		}
		warn!(LOGGER, "Please check the results of the migration process using `grin wallet info` and `grin wallet outputs`");
		warn!(LOGGER, "If anything went wrong, you can try again by deleting the `wallet_data` directory and running a wallet command");
		warn!(LOGGER, "If all is okay, you can move/backup/delete all files in the wallet directory EXCEPT FOR wallet.seed");
	}
	let client = HTTPWalletClient::new(&wallet_config.check_node_api_http_addr);
	let db_wallet = LMDBBackend::new(wallet_config.clone(), "", client).unwrap_or_else(|e| {
		panic!(
			"Error creating DB wallet: {} Config: {:?}",
			e, wallet_config
		);
	});
	info!(LOGGER, "Using LMDB Backend for wallet");
	Box::new(db_wallet)
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
		info!(LOGGER, "Wallet seed file created");
		let client = HTTPWalletClient::new(&wallet_config.check_node_api_http_addr);
		let _: LMDBBackend<HTTPWalletClient, keychain::ExtKeychain> =
			LMDBBackend::new(wallet_config.clone(), "", client).unwrap_or_else(|e| {
				panic!(
					"Error creating DB for wallet: {} Config: {:?}",
					e, wallet_config
				);
			});
		info!(LOGGER, "Wallet database backend created");
		// give logging thread a moment to catch up
		thread::sleep(Duration::from_millis(200));
		// we are done here with creating the wallet, so just return
		return;
	}

	let passphrase = wallet_args
		.value_of("pass")
		.expect("Failed to read passphrase.");

	// Handle listener startup commands
	{
		let wallet = instantiate_wallet(wallet_config.clone(), passphrase);
		match wallet_args.subcommand() {
			("listen", Some(listen_args)) => {
				if let Some(port) = listen_args.value_of("port") {
					wallet_config.api_listen_port = port.parse().unwrap();
				}
				wallet::controller::foreign_listener(wallet, &wallet_config.api_listen_addr())
					.unwrap_or_else(|e| {
						panic!(
							"Error creating wallet listener: {:?} Config: {:?}",
							e, wallet_config
						)
					});
			}
			("owner_api", Some(_api_args)) => {
				wallet::controller::owner_listener(wallet, "127.0.0.1:13420").unwrap_or_else(|e| {
					panic!(
						"Error creating wallet api listener: {:?} Config: {:?}",
						e, wallet_config
					)
				});
			}
			_ => {}
		};
	}

	// Handle single-use (command line) owner commands
	let wallet = Arc::new(Mutex::new(instantiate_wallet(
		wallet_config.clone(),
		passphrase,
	)));
	let res = wallet::controller::owner_single_use(wallet, |api| {
		match wallet_args.subcommand() {
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
				let result = api.issue_send_tx(
					amount,
					minimum_confirmations,
					dest,
					max_outputs,
					selection_strategy == "all",
				);
				let slate = match result {
					Ok(s) => {
						info!(
							LOGGER,
							"Tx created: {} grin to {} (strategy '{}')",
							amount_to_hr_string(amount),
							dest,
							selection_strategy,
						);
						s
					}
					Err(e) => {
						error!(LOGGER, "Tx not created: {:?}", e);
						match e.kind() {
							// user errors, don't backtrace
							libwallet::ErrorKind::NotEnoughFunds { .. } => {}
							libwallet::ErrorKind::FeeDispute { .. } => {}
							libwallet::ErrorKind::FeeExceedsAmount { .. } => {}
							_ => {
								// otherwise give full dump
								error!(LOGGER, "Backtrace: {}", e.backtrace().unwrap());
							}
						};
						panic!();
					}
				};
				let result = api.post_tx(&slate, fluff);
				match result {
					Ok(_) => {
						info!(LOGGER, "Tx sent",);
						Ok(())
					}
					Err(e) => {
						error!(LOGGER, "Tx not sent: {:?}", e);
						Err(e)
					}
				}
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
				api.issue_burn_tx(amount, minimum_confirmations, max_outputs)
					.unwrap_or_else(|e| {
						panic!("Error burning tx: {:?} Config: {:?}", e, wallet_config)
					});
				Ok(())
			}
			("info", Some(_)) => {
				let (validated, wallet_info) =
					api.retrieve_summary_info(true).unwrap_or_else(|e| {
						panic!(
							"Error getting wallet info: {:?} Config: {:?}",
							e, wallet_config
						)
					});
				wallet::display::info(&wallet_info, validated);
				Ok(())
			}
			("outputs", Some(_)) => {
				let (height, _) = api.node_height()?;
				let (validated, outputs) = api.retrieve_outputs(show_spent, true, None)?;
				let _res =
					wallet::display::outputs(height, validated, outputs).unwrap_or_else(|e| {
						panic!(
							"Error getting wallet outputs: {:?} Config: {:?}",
							e, wallet_config
						)
					});
				Ok(())
			}
			("txs", Some(txs_args)) => {
				let tx_id = match txs_args.value_of("id") {
					None => None,
					Some(tx) => match tx.parse() {
						Ok(t) => Some(t),
						Err(_) => panic!("Unable to parse argument 'id' as a number"),
					},
				};
				let (height, _) = api.node_height()?;
				let (validated, txs) = api.retrieve_txs(true, tx_id)?;
				let include_status = !tx_id.is_some();
				let _res = wallet::display::txs(height, validated, txs, include_status)
					.unwrap_or_else(|e| {
						panic!(
							"Error getting wallet outputs: {} Config: {:?}",
							e, wallet_config
						)
					});
				// if given a particular transaction id, also get and display associated
				// inputs/outputs
				if tx_id.is_some() {
					let (_, outputs) = api.retrieve_outputs(true, false, tx_id)?;
					let _res =
						wallet::display::outputs(height, validated, outputs).unwrap_or_else(|e| {
							panic!(
								"Error getting wallet outputs: {} Config: {:?}",
								e, wallet_config
							)
						});
				};
				Ok(())
			}
			("cancel", Some(tx_args)) => {
				let tx_id = tx_args
					.value_of("id")
					.expect("'id' argument (-i) is required.");
				let tx_id = tx_id.parse().expect("Could not parse id parameter.");
				let result = api.cancel_tx(tx_id);
				match result {
					Ok(_) => {
						info!(LOGGER, "Transaction {} Cancelled", tx_id);
						Ok(())
					}
					Err(e) => {
						error!(LOGGER, "TX Cancellation failed: {}", e);
						Err(e)
					}
				}
			}
			("restore", Some(_)) => {
				let result = api.restore();
				match result {
					Ok(_) => {
						info!(LOGGER, "Wallet restore complete",);
						Ok(())
					}
					Err(e) => {
						error!(LOGGER, "Wallet restore failed: {:?}", e);
						error!(LOGGER, "Backtrace: {}", e.backtrace().unwrap());
						Err(e)
					}
				}
			}
			_ => panic!("Unknown wallet command, use 'grin help wallet' for details"),
		}
	});
	// we need to give log output a chance to catch up before exiting
	thread::sleep(Duration::from_millis(100));

	if res.is_err() {
		exit(1);
	}
}
