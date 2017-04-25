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

fn main() {
	env_logger::init().unwrap();

	info!("Starting the Grin server...");
	grin::Server::start(read_config()).unwrap();

	loop {
		thread::sleep(Duration::from_secs(60));
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
