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
use grin_wallet as wallet;
use std::sync::Arc;
use std::{thread, time};

/// Start 1 node mining and two wallets, then send a few
/// transactions from one to the other
#[ignore]
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
	coinbase_config.wallet_validating_node_url = String::from("http://127.0.0.1:30001");
	coinbase_config.coinbase_wallet_address = String::from("http://127.0.0.1:13415");
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

	let mut server_config = LocalServerContainerConfig::default();
	server_config.name = String::from("server_one");
	server_config.p2p_server_port = 30000;
	server_config.api_server_port = 30001;
	server_config.start_miner = true;
	server_config.start_wallet = false;
	server_config.coinbase_wallet_address =
		String::from(format!("http://{}:{}", server_config.base_addr, 10002));
	// Spawn server and let it run for a bit
	let _ = thread::spawn(move || {
		let mut server_one = LocalServerContainer::new(server_config).unwrap();
		server_one.run_server(120);
	});

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
	LocalServerContainer::send_amount_to(
		&coinbase_wallet_config,
		"50.00",
		1,
		"not_all",
		"http://127.0.0.1:20002",
		false,
	);

	//Wait for a confirmation
	thread::sleep(time::Duration::from_millis(3000));
	let coinbase_info =
		LocalServerContainer::get_wallet_info(&coinbase_wallet_config, &coinbase_seed);
	println!("Coinbase wallet info: {:?}", coinbase_info);

	let recipient_info = LocalServerContainer::get_wallet_info(&recp_wallet_config, &recp_seed);
	println!("Recipient wallet info: {:?}", recipient_info);
	assert!(recipient_info.amount_currently_spendable == 50000000000);

	warn!("Sending many small transactions to recipient wallet");
	for _i in 0..10 {
		LocalServerContainer::send_amount_to(
			&coinbase_wallet_config,
			"1.00",
			1,
			"not_all",
			"http://127.0.0.1:20002",
			false,
		);
	}

	thread::sleep(time::Duration::from_millis(10000));
	let recipient_info = LocalServerContainer::get_wallet_info(&recp_wallet_config, &recp_seed);
	println!(
		"Recipient wallet info post little sends: {:?}",
		recipient_info
	);

	assert!(recipient_info.amount_currently_spendable == 60000000000);
	//send some cash right back
	LocalServerContainer::send_amount_to(
		&recp_wallet_config,
		"25.00",
		1,
		"all",
		"http://127.0.0.1:10002",
		false,
	);

	thread::sleep(time::Duration::from_millis(5000));

	let coinbase_info =
		LocalServerContainer::get_wallet_info(&coinbase_wallet_config, &coinbase_seed);
	println!("Coinbase wallet info final: {:?}", coinbase_info);
}

/// Tests the owner_api_include_foreign configuration option.
#[test]
fn wallet_config_owner_api_include_foreign() {
	// Test setup
	let test_name_dir = "test_servers";
	core::global::set_mining_mode(core::global::ChainTypes::AutomatedTesting);
	framework::clean_all_output(test_name_dir);
	let mut log_config = util::LoggingConfig::default();
	log_config.stdout_log_level = util::LogLevel::Info;
	util::init_test_logger();

	// This is just used for testing whether the API endpoint exists
	// so we have nonsense values
	let block_fees = wallet::BlockFees {
		fees: 1,
		height: 2,
		key_id: None,
	};

	let mut base_config = LocalServerContainerConfig::default();
	base_config.name = String::from("test_owner_api_include_foreign");
	base_config.start_wallet = true;

	// Start up the wallet owner API with the default config, and make sure
	// we get an error when trying to hit the coinbase endpoint
	let mut default_config = base_config.clone();
	default_config.owner_port = 20005;
	let _ = thread::spawn(move || {
		let mut default_owner = LocalServerContainer::new(default_config).unwrap();
		default_owner.run_owner();
	});
	thread::sleep(time::Duration::from_millis(1000));
	assert!(wallet::create_coinbase("http://127.0.0.1:20005", &block_fees).is_err());

	// Start up the wallet owner API with the owner_api_include_foreign setting set,
	// and confirm that we can hit the endpoint
	let mut foreign_config = base_config.clone();
	foreign_config.owner_port = 20006;
	foreign_config.owner_api_include_foreign = true;
	let _ = thread::spawn(move || {
		let mut owner_with_foreign = LocalServerContainer::new(foreign_config).unwrap();
		owner_with_foreign.run_owner();
	});
	thread::sleep(time::Duration::from_millis(1000));
	assert!(wallet::create_coinbase("http://127.0.0.1:20006", &block_fees).is_ok());
}
