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

#[macro_use]
extern crate log;

mod framework;

use self::util::Mutex;
use crate::framework::{LocalServerContainer, LocalServerContainerConfig};
use grin_core as core;
use grin_util as util;
use std::sync::Arc;
use std::{thread, time};

/// Start 1 node mining, 1 non mining node and two wallets.
/// Then send a transaction from one wallet to another and propagate it a stem
/// transaction but without stem relay and check if the transaction is still
/// broadcasted.
#[test]
#[ignore]
fn test_dandelion_timeout() {
	let test_name_dir = "test_dandelion_timeout";
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
	coinbase_config.wallet_validating_node_url = String::from("http://127.0.0.1:30001");
	coinbase_config.wallet_port = 10002;
	let coinbase_wallet = Arc::new(Mutex::new(
		LocalServerContainer::new(coinbase_config).unwrap(),
	));
	let coinbase_wallet_config = { coinbase_wallet.lock().wallet_config.clone() };

	let coinbase_seed = LocalServerContainer::get_wallet_seed(&coinbase_wallet_config);

	let _ = thread::spawn(move || {
		let mut w = coinbase_wallet.lock();
		w.run_wallet(0);
	});

	let mut recp_config = LocalServerContainerConfig::default();
	recp_config.name = String::from("target_wallet");
	recp_config.wallet_validating_node_url = String::from("http://127.0.0.1:30001");
	recp_config.wallet_port = 20002;
	let target_wallet = Arc::new(Mutex::new(LocalServerContainer::new(recp_config).unwrap()));
	let target_wallet_cloned = target_wallet.clone();
	let recp_wallet_config = { target_wallet.lock().wallet_config.clone() };

	let recp_seed = LocalServerContainer::get_wallet_seed(&recp_wallet_config);
	//Start up a second wallet, to receive
	let _ = thread::spawn(move || {
		let mut w = target_wallet_cloned.lock();
		w.run_wallet(0);
	});

	// Spawn server and let it run for a bit
	let mut server_one_config = LocalServerContainerConfig::default();
	server_one_config.name = String::from("server_one");
	server_one_config.p2p_server_port = 30000;
	server_one_config.api_server_port = 30001;
	server_one_config.start_miner = true;
	server_one_config.start_wallet = false;
	server_one_config.is_seeding = false;
	server_one_config.coinbase_wallet_address =
		String::from(format!("http://{}:{}", server_one_config.base_addr, 10002));
	let mut server_one = LocalServerContainer::new(server_one_config).unwrap();

	let mut server_two_config = LocalServerContainerConfig::default();
	server_two_config.name = String::from("server_two");
	server_two_config.p2p_server_port = 40000;
	server_two_config.api_server_port = 40001;
	server_two_config.start_miner = false;
	server_two_config.start_wallet = false;
	server_two_config.is_seeding = true;
	let mut server_two = LocalServerContainer::new(server_two_config.clone()).unwrap();

	server_one.add_peer(format!(
		"{}:{}",
		server_two_config.base_addr, server_two_config.p2p_server_port
	));

	// Spawn servers and let them run for a bit
	let _ = thread::spawn(move || {
		server_two.run_server(120);
	});

	// Wait for the first server to start
	thread::sleep(time::Duration::from_millis(5000));

	let _ = thread::spawn(move || {
		server_one.run_server(120);
	});

	// Let them do a handshake and properly update their peer relay
	thread::sleep(time::Duration::from_millis(30000));

	//Wait until we have some funds to send
	let mut coinbase_info =
		LocalServerContainer::get_wallet_info(&coinbase_wallet_config, &coinbase_seed);
	let mut slept_time = 0;
	while coinbase_info.amount_currently_spendable < 100000000000 {
		thread::sleep(time::Duration::from_millis(500));
		slept_time += 500;
		if slept_time > 10000 {
			panic!("Coinbase not confirming in time");
		}
		coinbase_info =
			LocalServerContainer::get_wallet_info(&coinbase_wallet_config, &coinbase_seed);
	}

	warn!("Sending 50 Grins to recipient wallet");

	// Sending stem transaction
	LocalServerContainer::send_amount_to(
		&coinbase_wallet_config,
		"50.00",
		1,
		"not_all",
		"http://127.0.0.1:20002",
		false,
	);

	let coinbase_info =
		LocalServerContainer::get_wallet_info(&coinbase_wallet_config, &coinbase_seed);
	println!("Coinbase wallet info: {:?}", coinbase_info);

	let recipient_info = LocalServerContainer::get_wallet_info(&recp_wallet_config, &recp_seed);

	// The transaction should be waiting in the node stempool thus cannot be mined.
	println!("Recipient wallet info: {:?}", recipient_info);
	assert!(recipient_info.amount_awaiting_confirmation == 50000000000);

	// Wait for stem timeout
	thread::sleep(time::Duration::from_millis(35000));
	println!("Recipient wallet info: {:?}", recipient_info);
	let recipient_info = LocalServerContainer::get_wallet_info(&recp_wallet_config, &recp_seed);
	assert!(recipient_info.amount_currently_spendable == 50000000000);
}
