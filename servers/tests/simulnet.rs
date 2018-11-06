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

use std::cmp;
use std::default::Default;
use std::process::exit;
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

/// Preparation:
/// 	Creates 5 disconnected servers: A, B, C, D, and E, mine 80 blocks on A,
/// 	Compact server A.
/// 	Connect all servers, check all get state_sync_threshold full blocks using fast sync.
/// 	Disconnect C, D and E from all others.
///
/// Test case 1: nodes that just synced is able to handle forks of up to state_sync_threshold
/// 	Mine state_sync_threshold-7 blocks on A, check server B get all blocks.
/// 	Mine state_sync_threshold-1 blocks on C (long fork), connect C to server A and B,
/// 	check server B can sync without txhashset download.
///
/// Test case 2: nodes with history in between state_sync_threshold and cut_through_horizon will
/// be able to handle forks larger than state_sync_threshold but not as large as cut_through_horizon.
/// 	Mine 30 blocks on A, check server B and C get all blocks.
/// 	Mine cut_through_horizon-1 blocks on D (longer fork), connect D to servers A, B and C,
/// 	check server B can sync without txhashset download.
///
/// Test case 3: nodes that have enough history is able to handle forks of up to cut_through_horizon
/// 	Mine cut_through_horizon-1 blocks on E, connect E to all other servers (A,B,C,D)
/// 	check server A can sync without txhashset download.
/// 	check server B can sync but need txhashset download.
///
#[test]
fn simulate_long_fork() {
	util::init_test_logger();
	println!("starting simulate_long_fork");

	// we actually set the chain_type in the ServerConfig below
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "grin-long-fork";
	framework::clean_all_output(test_name_dir);

	long_fork_test_preparation();
	thread::sleep(time::Duration::from_millis(5 * 1_000));

	long_fork_test_case_1();
	thread::sleep(time::Duration::from_millis(5 * 1_000));

	long_fork_test_case_2();
	thread::sleep(time::Duration::from_millis(5 * 1_000));

	// ------------------------------- //
	// ------    test case 3    ------ //
	// ------------------------------- //

	// TODO:
	// Mine cut_through_horizon-1 blocks on s4
	// Connect s4 to all other servers (s0,s1,s2,s3)
	// Check server s0 can sync without txhashset download.
	// Check server s1 can sync but need txhashset download.

	// wait servers fully stop before start next automated test
	thread::sleep(time::Duration::from_millis(1_000));
}

fn long_fork_test_preparation() {
	println!("preparation: mine 80 blocks");

	let mut s: Vec<servers::Server> = vec![];

	// start server A and mine 80 blocks to get beyond the fast sync horizon
	let mut conf = framework::config(2100, "grin-long-fork", 2100);
	conf.archive_mode = Some(false);
	conf.api_secret_path = None;
	let s0 = servers::Server::new(conf).unwrap();
	thread::sleep(time::Duration::from_millis(1_000));
	s.push(s0);
	let stop = Arc::new(AtomicBool::new(false));
	s[0].start_test_miner(None, stop.clone());

	while s[0].head().height < global::cut_through_horizon() as u64 + 10 {
		thread::sleep(time::Duration::from_millis(1_000));
	}
	s[0].stop_test_miner(stop);
	thread::sleep(time::Duration::from_millis(1_000));

	// Get the current header from s0.
	let s0_header = s[0].chain.head().unwrap();

	// check the tail after compacting
	let _ = s[0].chain.compact();
	let s0_tail = s[0].chain.tail().unwrap();
	assert_eq!(
		s0_header.height - global::cut_through_horizon() as u64,
		s0_tail.height
	);

	for i in 1..5 {
		let mut conf = config(2100 + i, "grin-long-fork", 2100);
		conf.archive_mode = Some(false);
		conf.api_secret_path = None;
		let si = servers::Server::new(conf).unwrap();
		s.push(si);
	}
	thread::sleep(time::Duration::from_millis(1_000));

	// Wait for s[1..4] to sync up to and including the header from s0.
	let mut total_wait = 0;
	let mut min_height = 0;
	while min_height < s0_header.height {
		thread::sleep(time::Duration::from_millis(1_000));
		total_wait += 1;
		if total_wait >= 60 {
			println!(
				"simulate_long_fork (preparation) test fail on timeout! minimum height: {}, s0 height: {}",
				min_height,
				s0_header.height,
			);
			exit(1);
		}
		min_height = s0_header.height;
		for i in 1..5 {
			min_height = cmp::min(s[i].head().height, min_height);
		}
	}

	// Confirm both s0 and s1 see a consistent header at that height.
	let s1_header = s[1].chain.head().unwrap();
	assert_eq!(s0_header, s1_header);

	// Wait for peers fully connection
	let mut total_wait = 0;
	let mut min_peers = 0;
	while min_peers < 4 {
		thread::sleep(time::Duration::from_millis(1_000));
		total_wait += 1;
		if total_wait >= 60 {
			println!(
				"simulate_long_fork (preparation) test fail on timeout! minimum connected peers: {}",
				min_peers,
			);
			exit(1);
		}
		min_peers = 4;
		for i in 0..5 {
			let peers_connected = get_connected_peers(&"127.0.0.1".to_owned(), 22100 + i);
			min_peers = cmp::min(min_peers, peers_connected.len());
		}
	}

	// Clean up
	for i in 0..5 {
		s[i].stop();
	}
}

fn long_fork_test_mining(blocks: u64, node: u16) {
	// Start server s[n]
	println!("s{} starting...", node);
	let mut conf = config(2100 + node, "grin-long-fork", 2100);
	conf.archive_mode = Some(false);
	conf.api_secret_path = None;
	let sn = servers::Server::new(conf).unwrap();
	thread::sleep(time::Duration::from_millis(1_000));
	println!("s{} started.", node);

	// Get the current header from node.
	let sn_header = sn.chain.head().unwrap();

	// Mine state_sync_threshold-7 blocks on s0
	let stop = Arc::new(AtomicBool::new(false));
	sn.start_test_miner(None, stop.clone());

	while sn.head().height < sn_header.height + blocks {
		thread::sleep(time::Duration::from_millis(1));
	}
	sn.stop_test_miner(stop);
	thread::sleep(time::Duration::from_millis(1_000));
	println!(
		"{} blocks mined on s{}. s{}.height: {}",
		blocks,
		node,
		node,
		sn.head().height,
	);

	let _ = sn.chain.compact();

	sn.stop();
}

fn long_fork_test_case_1() {
	println!("test case 1 start");

	// Mine state_sync_threshold-7 blocks on s0
	long_fork_test_mining(global::state_sync_threshold() as u64 - 7, 0);

	// Mine state_sync_threshold-1 blocks on s2 (long fork), a fork with more work than s0 chain
	long_fork_test_mining(global::state_sync_threshold() as u64 - 1, 2);

	println!("test case 1: s0 starting...");
	let mut conf = config(2100, "grin-long-fork", 2100);
	conf.archive_mode = Some(false);
	conf.api_secret_path = None;
	let s0 = servers::Server::new(conf).unwrap();
	thread::sleep(time::Duration::from_millis(1_000));
	let s0_header = s0.chain.head().unwrap();
	let s0_tail = s0.chain.tail().unwrap();
	println!(
		"test case 1: s0 started. s0.head().height: {}",
		s0_header.height
	);

	println!("test case 1: s2 starting...");
	let mut conf = config(2100 + 2, "grin-long-fork", 2100);
	conf.archive_mode = Some(false);
	conf.api_secret_path = None;
	let s2 = servers::Server::new(conf).unwrap();
	thread::sleep(time::Duration::from_millis(1_000));
	let s2_header = s2.chain.head().unwrap();
	println!(
		"test case 1: s2 started. s2.head().height: {}",
		s2_header.height
	);

	// Check server s0 can sync to s2 without txhashset download.
	let mut total_wait = 0;
	while s0.head().height < s2_header.height {
		thread::sleep(time::Duration::from_millis(1_000));
		total_wait += 1;
		if total_wait >= 60 {
			println!(
				"simulate_long_fork (t1) test fail on timeout! s1 height: {}, s2 height: {}",
				s0.head().height,
				s2_header.height,
			);
			exit(1);
		}
	}
	let s0_tail_new = s0.chain.tail().unwrap();
	assert_eq!(s0_tail_new.height, s0_tail.height);
	println!(
		"s0.head().height: {}, s2_header.height: {}",
		s0.head().height,
		s2_header.height,
	);
	assert_eq!(s0.head().last_block_h, s2_header.last_block_h);

	s0.stop();
	s2.stop();
}

fn long_fork_test_case_2() {
	println!("test case 2 start");

	// Mine 30 blocks on s0
	long_fork_test_mining(30, 0);

	// Mine cut_through_horizon-1 blocks on s3 (longer fork)
	long_fork_test_mining(global::cut_through_horizon() as u64 - 1, 3);

	println!("test case 2: s0 starting...");
	let mut conf = config(2100, "grin-long-fork", 2100);
	conf.archive_mode = Some(false);
	conf.api_secret_path = None;
	let s0 = servers::Server::new(conf).unwrap();
	thread::sleep(time::Duration::from_millis(1_000));
	let s0_header = s0.chain.head().unwrap();
	let s0_tail = s0.chain.tail().unwrap();
	println!(
		"test case 2: s0 started. s0.head().height: {}",
		s0_header.height
	);

	println!("test case 2: s3 starting...");
	let mut conf = config(2100 + 3, "grin-long-fork", 2100);
	conf.archive_mode = Some(false);
	conf.api_secret_path = None;
	let s3 = servers::Server::new(conf).unwrap();
	thread::sleep(time::Duration::from_millis(1_000));
	let s3_header = s3.chain.head().unwrap();
	println!(
		"test case 2: s3 started. s3.head().height: {}",
		s3_header.height
	);

	// Check server s0 can sync to s3 without txhashset download.
	let mut total_wait = 0;
	while s0.head().height < s3_header.height {
		thread::sleep(time::Duration::from_millis(1_000));
		total_wait += 1;
		if total_wait >= 120 {
			println!(
				"simulate_long_fork (t2) test fail on timeout! s0 height: {}, s3 height: {}",
				s0.head().height,
				s3_header.height,
			);
			exit(1);
		}
	}
	let s0_tail_new = s0.chain.tail().unwrap();
	assert_eq!(s0_tail_new.height, s0_tail.height);
	assert_eq!(s0.head().hash(), s3_header.hash());

	s0.stop();
	s3.stop();
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

fn ban_peer(base_addr: &String, api_server_port: u16, peer_addr: &String) {
	let url = format!(
		"http://{}:{}/v1/peers/{}/ban",
		base_addr, api_server_port, peer_addr
	);
	println!("ban_peer: url={}", url);
	let _ = api::client::post_no_ret(url.as_str(), None, &"");
}

fn unban_peer(base_addr: &String, api_server_port: u16, peer_addr: &String) {
	let url = format!(
		"http://{}:{}/v1/peers/{}/unban",
		base_addr, api_server_port, peer_addr
	);
	println!("unban_peer: url={}", url);
	let _ = api::client::post_no_ret(url.as_str(), None, &"");
}

fn get_connected_peers(
	base_addr: &String,
	api_server_port: u16,
) -> Vec<p2p::types::PeerInfoDisplay> {
	let url = format!(
		"http://{}:{}/v1/peers/connected",
		base_addr, api_server_port
	);
	api::client::get::<Vec<p2p::types::PeerInfoDisplay>>(url.as_str(), None).unwrap()
}
