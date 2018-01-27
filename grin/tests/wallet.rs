// Copyright 2017 The Grin Developers
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

#[macro_use]
extern crate router;
#[macro_use]
extern crate slog;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_grin as grin;
extern crate grin_p2p as p2p;
extern crate grin_pow as pow;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate grin_config as config;

mod framework;

use std::{thread, time};
use std::sync::{Arc, Mutex};
use framework::{LocalServerContainer,LocalServerContainerConfig};

use util::LOGGER;

/// Start 1 node mining and two wallets, then send a few
/// transactions from one to the other
#[test]
fn basic_wallet_transactions() {
	let test_name_dir = "test_servers";
	core::global::set_mining_mode(core::global::ChainTypes::AutomatedTesting);
	framework::clean_all_output(test_name_dir);
	let mut log_config = util::LoggingConfig::default();
	//log_config.stdout_log_level = util::LogLevel::Trace;
	log_config.stdout_log_level = util::LogLevel::Info;
	//init_logger(Some(log_config));
	util::init_test_logger();

	// Run a separate coinbase wallet for coinbase transactions
	let mut coinbase_config = LocalServerContainerConfig::default();
	coinbase_config.name = String::from("coinbase_wallet");
	coinbase_config.wallet_validating_node_url=String::from("http://127.0.0.1:30001");
	coinbase_config.wallet_port = 10002;
	let coinbase_wallet = Arc::new(Mutex::new(LocalServerContainer::new(coinbase_config).unwrap()));
	let coinbase_wallet_config = {
		coinbase_wallet.lock().unwrap().wallet_config.clone()
	};

	let _ = thread::spawn(move || {
		let mut w = coinbase_wallet.lock().unwrap();
		w.run_wallet(0);
	});

	let mut recp_config = LocalServerContainerConfig::default();
	recp_config.name = String::from("target_wallet");
	recp_config.wallet_validating_node_url=String::from("http://127.0.0.1:30001");
	recp_config.wallet_port = 20002;
	let target_wallet = Arc::new(Mutex::new(LocalServerContainer::new(recp_config).unwrap()));
	let target_wallet_cloned = target_wallet.clone();
	let recp_wallet_config = {
		target_wallet.lock().unwrap().wallet_config.clone()
	};

	//Start up a second wallet, to receive
	let _ = thread::spawn(move || {
		let mut w = target_wallet_cloned.lock().unwrap();
		w.run_wallet(0);
	});

	// Spawn server and let it run for a bit
	let _ = thread::spawn(move || {
		let mut server_config = LocalServerContainerConfig::default();
		server_config.name = String::from("server_one");
		server_config.p2p_server_port = 30000;
		server_config.api_server_port = 30001;
		server_config.start_miner = true;
		server_config.start_wallet = false;
		server_config.coinbase_wallet_address = String::from(format!(
			"http://{}:{}",
			server_config.base_addr,
			10002
		));
		let mut server_one = LocalServerContainer::new(server_config).unwrap();
		server_one.run_server(120);
	});

	//Wait for chain to build
	thread::sleep(time::Duration::from_millis(5000));
	warn!(LOGGER, "Sending 50 Grins to recipient wallet");
	LocalServerContainer::send_amount_to(&coinbase_wallet_config, "50.00", 1, "all", "http://127.0.0.1:20002");

	//let some more mining happen, make sure nothing pukes
	thread::sleep(time::Duration::from_millis(5000));

	//send some cash right back
	LocalServerContainer::send_amount_to(&recp_wallet_config, "25.00", 1, "all", "http://127.0.0.1:10002");
	thread::sleep(time::Duration::from_millis(5000));
}
