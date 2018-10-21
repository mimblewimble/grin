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

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_servers as servers;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
#[macro_use]
extern crate log;

mod framework;

use std::default::Default;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::{thread, time};
use util::Mutex;

use core::core::hash::Hashed;
use core::global::{self, ChainTypes};

use wallet::controller;
use wallet::libtx::slate::Slate;
use wallet::libwallet::types::{WalletBackend, WalletInst};
use wallet::lmdb_wallet::LMDBBackend;
use wallet::HTTPWalletClient;
use wallet::WalletConfig;

use framework::{
	config, stop_all_servers, LocalServerContainerConfig, LocalServerContainerPool,
	LocalServerContainerPoolConfig,
};

/// Testing the frameworks by starting a fresh server, creating a genesis
/// Block and mining into a wallet for a bit
#[test]
fn basic_genesis_mine() {
	util::init_test_logger();
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "genesis_mine";
	framework::clean_all_output(test_name_dir);

	// Create a server pool
	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = String::from(test_name_dir);
	pool_config.run_length_in_seconds = 10;

	pool_config.base_api_port = 30000;
	pool_config.base_p2p_port = 31000;
	pool_config.base_wallet_port = 32000;

	let mut pool = LocalServerContainerPool::new(pool_config);

	// Create a server to add into the pool
	let mut server_config = LocalServerContainerConfig::default();
	server_config.start_miner = true;
	server_config.start_wallet = false;
	server_config.burn_mining_rewards = true;

	pool.create_server(&mut server_config);
	let servers = pool.run_all_servers();
	stop_all_servers(servers);
}

/// Creates 5 servers, first being a seed and check that through peer address
/// messages they all end up connected.
#[test]
fn simulate_seeding() {
	util::init_test_logger();
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "simulate_seeding";
	framework::clean_all_output(test_name_dir);

	// Create a server pool
	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = test_name_dir.to_string();
	pool_config.run_length_in_seconds = 30;

	// have to use different ports because of tests being run in parallel
	pool_config.base_api_port = 30020;
	pool_config.base_p2p_port = 31020;
	pool_config.base_wallet_port = 32020;

	let mut pool = LocalServerContainerPool::new(pool_config);

	// Create a first seed server to add into the pool
	let mut server_config = LocalServerContainerConfig::default();
	// server_config.start_miner = true;
	server_config.start_wallet = false;
	server_config.burn_mining_rewards = true;
	server_config.is_seeding = true;

	pool.create_server(&mut server_config);

	// wait the seed server fully start up before start remaining servers
	thread::sleep(time::Duration::from_millis(1_000));

	// point next servers at first seed
	server_config.is_seeding = false;
	server_config.seed_addr = format!(
		"{}:{}",
		server_config.base_addr, server_config.p2p_server_port
	);

	for _ in 0..4 {
		pool.create_server(&mut server_config);
	}

	let servers = pool.run_all_servers();
	thread::sleep(time::Duration::from_secs(5));

	// Check they all end up connected.
	let url = format!(
		"http://{}:{}/v1/peers/connected",
		&server_config.base_addr, 30020
	);
	let peers_all = api::client::get::<Vec<p2p::types::PeerInfoDisplay>>(url.as_str(), None);
	assert!(peers_all.is_ok());
	assert_eq!(peers_all.unwrap().len(), 4);

	stop_all_servers(servers);

	// wait servers fully stop before start next automated test
	thread::sleep(time::Duration::from_millis(1_000));
}

/// Create 1 server, start it mining, then connect 4 other peers mining and
/// using the first as a seed. Meant to test the evolution of mining difficulty with miners
/// running at different rates.
///
/// TODO: Just going to comment this out as an automatically run test for the time
/// being, As it's more for actively testing and hurts CI a lot
#[ignore]
#[test]
fn simulate_parallel_mining() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "simulate_parallel_mining";
	// framework::clean_all_output(test_name_dir);

	// Create a server pool
	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = test_name_dir.to_string();
	pool_config.run_length_in_seconds = 60;
	// have to use different ports because of tests being run in parallel
	pool_config.base_api_port = 30040;
	pool_config.base_p2p_port = 31040;
	pool_config.base_wallet_port = 32040;

	let mut pool = LocalServerContainerPool::new(pool_config);

	// Create a first seed server to add into the pool
	let mut server_config = LocalServerContainerConfig::default();
	server_config.start_miner = true;
	server_config.start_wallet = true;
	server_config.is_seeding = true;

	pool.create_server(&mut server_config);

	// point next servers at first seed
	server_config.is_seeding = false;
	server_config.seed_addr = format!(
		"{}:{}",
		server_config.base_addr, server_config.p2p_server_port
	);

	// And create 4 more, then let them run for a while
	for i in 1..4 {
		// fudge in some slowdown
		server_config.miner_slowdown_in_millis = i * 2;
		pool.create_server(&mut server_config);
	}

	// pool.connect_all_peers();

	let servers = pool.run_all_servers();
	stop_all_servers(servers);

	// Check mining difficulty here?, though I'd think it's more valuable
	// to simply output it. Can at least see the evolution of the difficulty target
	// in the debug log output for now
}

// TODO: Convert these tests to newer framework format
/// Create a network of 5 servers and mine a block, verifying that the block
/// gets propagated to all.
#[test]
fn simulate_block_propagation() {
	util::init_test_logger();

	// we actually set the chain_type in the ServerConfig below
	// TODO - avoid needing to set it in two places?
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "grin-prop";
	framework::clean_all_output(test_name_dir);

	// instantiates 5 servers on different ports
	let mut servers = vec![];
	for n in 0..5 {
		let s = servers::Server::new(framework::config(10 * n, test_name_dir, 0)).unwrap();
		servers.push(s);
		thread::sleep(time::Duration::from_millis(100));
	}

	// start mining
	let stop = Arc::new(AtomicBool::new(false));
	servers[0].start_test_miner(None, stop.clone());

	// monitor for a change of head on a different server and check whether
	// chain height has changed
	let mut success = false;
	let mut time_spent = 0;
	loop {
		let mut count = 0;
		for n in 0..5 {
			if servers[n].head().height > 3 {
				count += 1;
			}
		}
		if count == 5 {
			success = true;
			break;
		}
		thread::sleep(time::Duration::from_millis(1_000));
		time_spent += 1;
		if time_spent >= 30 {
			info!("simulate_block_propagation - fail on timeout",);
			break;
		}

		// stop mining after 8s
		if time_spent == 8 {
			servers[0].stop_test_miner(stop.clone());
		}
	}
	for n in 0..5 {
		servers[n].stop();
	}
	assert_eq!(true, success);

	// wait servers fully stop before start next automated test
	thread::sleep(time::Duration::from_millis(1_000));
}

/// Creates 2 different disconnected servers, mine a few blocks on one, connect
/// them and check that the 2nd gets all the blocks
#[test]
fn simulate_full_sync() {
	util::init_test_logger();

	// we actually set the chain_type in the ServerConfig below
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "grin-sync";
	framework::clean_all_output(test_name_dir);

	let s1 = servers::Server::new(framework::config(1000, "grin-sync", 1000)).unwrap();
	// mine a few blocks on server 1
	let stop = Arc::new(AtomicBool::new(false));
	s1.start_test_miner(None, stop.clone());
	thread::sleep(time::Duration::from_secs(8));
	s1.stop_test_miner(stop);

	let s2 = servers::Server::new(framework::config(1001, "grin-sync", 1000)).unwrap();

	// Get the current header from s1.
	let s1_header = s1.chain.head_header().unwrap();
	info!(
		"simulate_full_sync - s1 header head: {} at {}",
		s1_header.hash(),
		s1_header.height
	);

	// Wait for s2 to sync up to and including the header from s1.
	let mut time_spent = 0;
	while s2.head().height < s1_header.height {
		thread::sleep(time::Duration::from_millis(1_000));
		time_spent += 1;
		if time_spent >= 30 {
			info!(
				"sync fail. s2.head().height: {}, s1_header.height: {}",
				s2.head().height,
				s1_header.height
			);
			break;
		}
	}

	// Confirm both s1 and s2 see a consistent header at that height.
	let s2_header = s2.chain.get_block_header(&s1_header.hash()).unwrap();
	assert_eq!(s1_header, s2_header);

	// Stop our servers cleanly.
	s1.stop();
	s2.stop();

	// wait servers fully stop before start next automated test
	thread::sleep(time::Duration::from_millis(1_000));
}

/// Creates 2 different disconnected servers, mine a few blocks on one, connect
/// them and check that the 2nd gets all using fast sync algo
#[test]
fn simulate_fast_sync() {
	util::init_test_logger();

	// we actually set the chain_type in the ServerConfig below
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "grin-fast";
	framework::clean_all_output(test_name_dir);

	// start s1 and mine enough blocks to get beyond the fast sync horizon
	let s1 = servers::Server::new(framework::config(2000, "grin-fast", 2000)).unwrap();
	let stop = Arc::new(AtomicBool::new(false));
	s1.start_test_miner(None, stop.clone());

	while s1.head().height < 20 {
		thread::sleep(time::Duration::from_millis(1_000));
	}
	s1.stop_test_miner(stop);

	let mut conf = config(2001, "grin-fast", 2000);
	conf.archive_mode = Some(false);

	let s2 = servers::Server::new(conf).unwrap();

	// Get the current header from s1.
	let s1_header = s1.chain.head_header().unwrap();

	// Wait for s2 to sync up to and including the header from s1.
	let mut total_wait = 0;
	while s2.head().height < s1_header.height {
		thread::sleep(time::Duration::from_millis(1_000));
		total_wait += 1;
		if total_wait >= 30 {
			error!(
				"simulate_fast_sync test fail on timeout! s2 height: {}, s1 height: {}",
				s2.head().height,
				s1_header.height,
			);
			break;
		}
	}

	// Confirm both s1 and s2 see a consistent header at that height.
	let s2_header = s2.chain.get_block_header(&s1_header.hash()).unwrap();
	assert_eq!(s1_header, s2_header);

	// Stop our servers cleanly.
	s1.stop();
	s2.stop();

	// wait servers fully stop before start next automated test
	thread::sleep(time::Duration::from_millis(1_000));
}

#[ignore]
#[test]
fn simulate_fast_sync_double() {
	util::init_test_logger();

	// we actually set the chain_type in the ServerConfig below
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	framework::clean_all_output("grin-double-fast1");
	framework::clean_all_output("grin-double-fast2");

	let s1 = servers::Server::new(framework::config(3000, "grin-double-fast1", 3000)).unwrap();
	// mine a few blocks on server 1
	s1.start_test_miner(None, s1.stop.clone());
	thread::sleep(time::Duration::from_secs(8));

	{
		let mut conf = config(3001, "grin-double-fast2", 3000);
		conf.archive_mode = Some(false);
		let s2 = servers::Server::new(conf).unwrap();
		while s2.head().height != s2.header_head().height || s2.head().height < 20 {
			thread::sleep(time::Duration::from_millis(1000));
		}
		s2.stop();
	}
	// locks files don't seem to be cleaned properly until process exit
	std::fs::remove_file("target/tmp/grin-double-fast2/grin-sync-1001/chain/LOCK").unwrap();
	std::fs::remove_file("target/tmp/grin-double-fast2/grin-sync-1001/peers/LOCK").unwrap();
	thread::sleep(time::Duration::from_secs(20));

	let mut conf = config(3001, "grin-double-fast2", 3000);
	conf.archive_mode = Some(false);
	let s2 = servers::Server::new(conf).unwrap();
	while s2.head().height != s2.header_head().height || s2.head().height < 50 {
		thread::sleep(time::Duration::from_millis(1000));
	}
	s1.stop();
	s2.stop();

	// wait servers fully stop before start next automated test
	thread::sleep(time::Duration::from_millis(1_000));
}

pub fn create_wallet(
	dir: &str,
	client: HTTPWalletClient,
) -> Box<WalletInst<HTTPWalletClient, keychain::ExtKeychain>> {
	let mut wallet_config = WalletConfig::default();
	wallet_config.data_file_dir = String::from(dir);
	let _ = wallet::WalletSeed::init_file(&wallet_config);
	let mut wallet: LMDBBackend<HTTPWalletClient, keychain::ExtKeychain> =
		LMDBBackend::new(wallet_config.clone(), "", client).unwrap_or_else(|e| {
			panic!("Error creating wallet: {:?} Config: {:?}", e, wallet_config)
		});
	wallet.open_with_credentials().unwrap_or_else(|e| {
		panic!(
			"Error initializing wallet: {:?} Config: {:?}",
			e, wallet_config
		)
	});
	Box::new(wallet)
}

/// Intended to replicate https://github.com/mimblewimble/grin/issues/1325
#[ignore]
#[test]
fn replicate_tx_fluff_failure() {
	util::init_test_logger();
	global::set_mining_mode(ChainTypes::UserTesting);
	framework::clean_all_output("tx_fluff");

	// Create Wallet 1 (Mining Input) and start it listening
	// Wallet 1 post to another node, just for fun
	let client1 = HTTPWalletClient::new("http://127.0.0.1:23003", None);
	let wallet1 = create_wallet("target/tmp/tx_fluff/wallet1", client1.clone());
	let wallet1_handle = thread::spawn(move || {
		controller::foreign_listener(wallet1, "127.0.0.1:33000", None)
			.unwrap_or_else(|e| panic!("Error creating wallet1 listener: {:?}", e,));
	});

	// Create Wallet 2 (Recipient) and launch
	let client2 = HTTPWalletClient::new("http://127.0.0.1:23001", None);
	let wallet2 = create_wallet("target/tmp/tx_fluff/wallet2", client2.clone());
	let wallet2_handle = thread::spawn(move || {
		controller::foreign_listener(wallet2, "127.0.0.1:33001", None)
			.unwrap_or_else(|e| panic!("Error creating wallet2 listener: {:?}", e,));
	});

	// Server 1 (mines into wallet 1)
	let mut s1_config = framework::config(3000, "tx_fluff", 3000);
	s1_config.test_miner_wallet_url = Some("http://127.0.0.1:33000".to_owned());
	s1_config.dandelion_config.embargo_secs = Some(10);
	s1_config.dandelion_config.patience_secs = Some(1);
	s1_config.dandelion_config.relay_secs = Some(1);
	let s1 = servers::Server::new(s1_config.clone()).unwrap();
	// Mine off of server 1
	s1.start_test_miner(s1_config.test_miner_wallet_url, s1.stop.clone());
	thread::sleep(time::Duration::from_secs(5));

	// Server 2 (another node)
	let mut s2_config = framework::config(3001, "tx_fluff", 3001);
	s2_config.p2p_config.seeds = Some(vec!["127.0.0.1:13000".to_owned()]);
	s2_config.dandelion_config.embargo_secs = Some(10);
	s2_config.dandelion_config.patience_secs = Some(1);
	s2_config.dandelion_config.relay_secs = Some(1);
	let _s2 = servers::Server::new(s2_config.clone()).unwrap();

	let dl_nodes = 5;

	for i in 0..dl_nodes {
		// (create some stem nodes)
		let mut s_config = framework::config(3002 + i, "tx_fluff", 3002 + i);
		s_config.p2p_config.seeds = Some(vec!["127.0.0.1:13000".to_owned()]);
		s_config.dandelion_config.embargo_secs = Some(10);
		s_config.dandelion_config.patience_secs = Some(1);
		s_config.dandelion_config.relay_secs = Some(1);
		let _ = servers::Server::new(s_config.clone()).unwrap();
	}

	thread::sleep(time::Duration::from_secs(10));

	// get another instance of wallet1 (to update contents and perform a send)
	let wallet1 = create_wallet("target/tmp/tx_fluff/wallet1", client1.clone());
	let wallet1 = Arc::new(Mutex::new(wallet1));

	let amount = 30_000_000_000;
	let mut slate = Slate::blank(1);

	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		slate = api
			.issue_send_tx(
				amount,                   // amount
				2,                        // minimum confirmations
				"http://127.0.0.1:33001", // dest
				500,                      // max outputs
				1000,                     // num change outputs
				true,                     // select all outputs
			).unwrap();
		api.post_tx(&slate, false).unwrap();
		Ok(())
	}).unwrap();

	// Give some time for propagation and mining
	thread::sleep(time::Duration::from_secs(200));

	// get another instance of wallet (to check contents)
	let wallet2 = create_wallet("target/tmp/tx_fluff/wallet2", client2.clone());

	let wallet2 = Arc::new(Mutex::new(wallet2));
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let res = api.retrieve_summary_info(true).unwrap();
		assert_eq!(res.1.amount_currently_spendable, amount);
		Ok(())
	}).unwrap();
}
