// Copyright 2016 The Grin Developers
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

extern crate clap;
extern crate daemonize;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate serde;
extern crate serde_json;
extern crate tiny_keccak;

extern crate grin_api as api;
extern crate grin_grin as grin;
extern crate grin_wallet as wallet;
extern crate secp256k1zkp as secp;

const GRIN_HOME: &'static str = ".grin";

use std::env;
use std::thread;
use std::io::Read;
use std::fs::File;
use std::time::Duration;
use tiny_keccak::Keccak;

use clap::{Arg, App, SubCommand, ArgMatches};
use daemonize::Daemonize;

use secp::Secp256k1;

use wallet::WalletConfig;

fn main() {
	env_logger::init().unwrap();

	let args = App::new("Grin")
    .version("0.1")
    .author("The Grin Team")
    .about("Lightweight implementation of the MimbleWimble protocol.")

    // specification of all the server commands and options
    .subcommand(SubCommand::with_name("server")
                .about("Control the Grin server")
                .arg(Arg::with_name("port")
                     .short("p")
                     .long("port")
                     .help("Port to start the server on")
                     .takes_value(true))
                .arg(Arg::with_name("seed")
                     .short("s")
                     .long("seed")
                     .help("Override seed node(s) to connect to")
                     .takes_value(true)
                     .multiple(true))
                .arg(Arg::with_name("mine")
                     .short("m")
                     .long("mine")
                     .help("Starts the debugging mining loop"))
				.arg(Arg::with_name("wallet_url")
                     .short("w")
                     .long("wallet_url")
                     .help("A listening wallet receiver to which mining rewards will be sent")
					 .takes_value(true))
                .arg(Arg::with_name("config")
                     .short("c")
                     .long("config")
                     .value_name("FILE.json")
                     .help("Sets a custom json configuration file")
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
                            .about("current status of the Grin chain")))

    // specification of the wallet commands and options
    .subcommand(SubCommand::with_name("wallet")
                .about("Wallet software for Grin")
                .arg(Arg::with_name("pass")
                     .short("p")
                     .long("pass")
                     .help("Wallet passphrase used to generate the private key seed")
                     .takes_value(true))
				.arg(Arg::with_name("dir")
                     .short("d")
                     .long("dir")
                     .help("Directory in which to store wallet files (defaults to current directory)")
                     .takes_value(true))
				.arg(Arg::with_name("port")
                     .short("r")
                     .long("port")
                     .help("Port on which to run the wallet receiver when in receiver mode")
                     .takes_value(true))	 	 
                .subcommand(SubCommand::with_name("receive")
                            .about("Run the wallet in receiving mode. If an input file is provided, will process it, otherwise runs in server mode waiting for send requests.")
                            .arg(Arg::with_name("input")
                                 .help("Partial transaction to receive, expects as a JSON file.")
                                 .short("i")
                                 .long("input")
                                 .takes_value(true)))
                .subcommand(SubCommand::with_name("send")
                            .about("Builds a transaction to send someone some coins. By default, the transaction will just be printed to stdout. If a destination is provided, the command will attempt to contact the receiver at that address and send the transaction directly.")
                            .arg(Arg::with_name("amount")
                                 .help("Amount to send in the smallest denomination")
                                 .index(1))
                            .arg(Arg::with_name("dest")
                                 .help("Send the transaction to the provided server")
                                 .short("d")
                                 .long("dest")
                                 .takes_value(true))))
    .get_matches();

	match args.subcommand() {
		// server commands and options
		("server", Some(server_args)) => {
			server_command(server_args);
		}

		// client commands and options
		("client", Some(client_args)) => {
			match client_args.subcommand() {
				("status", _) => {
					println!("status info...");
				}
				_ => panic!("Unknown client command, use 'grin help client' for details"),
			}
		}

		// client commands and options
		("wallet", Some(wallet_args)) => {
			wallet_command(wallet_args);
		}

		_ => println!("Unknown command, use 'grin help' for a list of all commands"),
	}
}

/// Handles the server part of the command line, mostly running, starting and
/// stopping the Grin blockchain server. Processes all the command line
/// arguments
/// to build a proper configuration and runs Grin with that configuration.
fn server_command(server_args: &ArgMatches) {
	info!("Starting the Grin server...");

	// configuration wrangling
	let mut server_config = read_config();
	if let Some(port) = server_args.value_of("port") {
		server_config.p2p_config.port = port.parse().unwrap();
	}
	if server_args.is_present("mine") {
		server_config.mining_config.enable_mining = true;
	}
	if let Some(wallet_url) = server_args.value_of("wallet_url") {
		server_config.mining_config.wallet_receiver_url = wallet_url.to_string();
	}

	if let Some(seeds) = server_args.values_of("seed") {
		server_config.seeding_type = grin::Seeding::List(seeds.map(|s| s.to_string()).collect());
	}

	// start the server in the different run modes (interactive or daemon)
	match server_args.subcommand() {
		("run", _) => {
			grin::Server::start(server_config).unwrap();
			loop {
				thread::sleep(Duration::from_secs(60));
			}
		}
		("start", _) => {
			let daemonize = Daemonize::new()
				.pid_file("/tmp/grin.pid")
				.chown_pid_file(true)
				.privileged_action(move || {
					grin::Server::start(server_config.clone()).unwrap();
					loop {
						thread::sleep(Duration::from_secs(60));
					}
				});
			match daemonize.start() {
				Ok(_) => info!("Grin server succesfully started."),
				Err(e) => error!("Error starting: {}", e),
			}
		}
		("stop", _) => println!("TODO, just 'kill $pid' for now."),
		_ => panic!("Unknown server command, use 'grin help server' for details"),
	}
}

fn wallet_command(wallet_args: &ArgMatches) {
	let hd_seed = wallet_args.value_of("pass").expect("Wallet passphrase required.");

	// TODO do something closer to BIP39, eazy solution right now
	let mut sha3 = Keccak::new_sha3_256();
	sha3.update(hd_seed.as_bytes());
	let mut seed = [0; 32];
	sha3.finalize(&mut seed);

	let s = Secp256k1::new();
	let key = wallet::ExtendedKey::from_seed(&s, &seed[..])
		.expect("Error deriving extended key from seed.");

	let default_ip = "127.0.0.1";
	let mut addr = format!("{}:13416", default_ip);

	let mut wallet_config = WalletConfig::default();
	if let Some(port) = wallet_args.value_of("port") {
		addr = format!("{}:{}", default_ip, port);
		wallet_config.api_http_addr = format!("http://{}", addr).to_string();
	}

	if let Some(dir) = wallet_args.value_of("dir") {
		wallet_config.data_file_dir = dir.to_string().clone();
	}
	
	match wallet_args.subcommand() {
		
		("receive", Some(receive_args)) => {
			if let Some(f) = receive_args.value_of("input") {
				let mut file = File::open(f).expect("Unable to open transaction file.");
				let mut contents = String::new();
				file.read_to_string(&mut contents).expect("Unable to read transaction file.");
				wallet::receive_json_tx(&wallet_config, &key, contents.as_str()).unwrap();
			} else {
				info!("Starting the Grin wallet receiving daemon at {}...", wallet_config.api_http_addr);
				let mut apis = api::ApiServer::new("/v1".to_string());
				apis.register_endpoint("/receive".to_string(), wallet::WalletReceiver { 
					key: key,
					config: wallet_config
				});
				apis.start(addr).unwrap_or_else(|e| {
					error!("Failed to start Grin wallet receiver: {}.", e);
				});
			}
		}
		("send", Some(send_args)) => {
			let amount = send_args.value_of("amount")
				.expect("Amount to send required")
				.parse()
				.expect("Could not parse amount as a whole number.");
			let mut dest = "stdout";
			if let Some(d) = send_args.value_of("dest") {
				dest = d;
			}
			wallet::issue_send_tx(&wallet_config, &key, amount, dest.to_string()).unwrap();
		}
		_ => panic!("Unknown wallet command, use 'grin help wallet' for details"),
	}
}

fn read_config() -> grin::ServerConfig {
	let mut config_path = env::home_dir().ok_or("Failed to detect home directory!").unwrap();
	config_path.push(GRIN_HOME);
	if !config_path.exists() {
		return default_config();
	}
	let mut config_file = File::open(config_path).unwrap();
	let mut config_content = String::new();
	config_file.read_to_string(&mut config_content).unwrap();
	serde_json::from_str(config_content.as_str()).unwrap()
}

fn default_config() -> grin::ServerConfig {
	grin::ServerConfig {
    test_mode: true,
		seeding_type: grin::Seeding::WebStatic,
		..Default::default()
	}
}
