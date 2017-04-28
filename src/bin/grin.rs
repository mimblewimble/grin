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

extern crate grin_grin as grin;

const GRIN_HOME: &'static str = ".grin";

use std::env;
use std::thread;
use std::io::Read;
use std::fs::File;
use std::time::Duration;

use clap::{Arg, App, SubCommand, ArgMatches};
use daemonize::Daemonize;

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
    .get_matches();

  match args.subcommand() {
    // server commands and options
    ("server", Some(server_args)) => {
      server_command(server_args);
    },

    // client commands and options
    ("client", Some(client_args)) => {
      match client_args.subcommand() {
        ("status", _) => {
          println!("status info...");
        },
        _ => panic!("Unknown client command, use 'grin help client' for details"),
      }
    }

    _ => panic!("Unknown command, use 'grin help' for a list of all commands"),
  }
}

/// Handles the server part of the command line, mostly running, starting and
/// stopping the Grin blockchain server. Processes all the command line arguments
/// to build a proper configuration and runs Grin with that configuration.
fn server_command(server_args: &ArgMatches) {
  info!("Starting the Grin server...");

  // configuration wrangling
  let mut server_config = read_config();
  if let Some(port) = server_args.value_of("port") {
    server_config.p2p_config.port = port.parse().unwrap();
  }
  if server_args.is_present("mine") {
    server_config.enable_mining = true;
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
    },
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
    ("stop", _) => {
      println!("TODO, just 'kill $pid' for now.")
    }
    _ => panic!("Unknown server command, use 'grin help server' for details"),
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
		cuckoo_size: 12,
		seeding_type: grin::Seeding::WebStatic,
		enable_mining: false,
		..Default::default()
	}
}
