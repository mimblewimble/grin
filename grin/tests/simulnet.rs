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
extern crate grin_grin as grin;
extern crate grin_p2p as p2p;
extern crate grin_pow as pow;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

mod framework;

use std::fs;
use std::sync::Arc;
use std::thread;
use std::time;
use std::default::Default;

use core::global;
use core::global::ChainTypes;

use framework::{LocalServerContainerConfig, LocalServerContainerPool,
                LocalServerContainerPoolConfig};

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
	pool.run_all_servers();
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
	pool_config.base_name = String::from(test_name_dir);
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

	// point next servers at first seed
	server_config.is_seeding = false;
	server_config.seed_addr = String::from(format!(
		"{}:{}",
		server_config.base_addr, server_config.p2p_server_port
	));

	for _ in 0..4 {
		pool.create_server(&mut server_config);
	}

	// pool.connect_all_peers();

	let _ = pool.run_all_servers();
}

/// Create 1 server, start it mining, then connect 4 other peers mining and
/// using the first
/// as a seed. Meant to test the evolution of mining difficulty with miners
/// running at
/// different rates
// Just going to comment this out as an automatically run test for the time
// being,
// As it's more for actively testing and hurts CI a lot
//#[test]
#[allow(dead_code)]
fn simulate_parallel_mining() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "simulate_parallel_mining";
	// framework::clean_all_output(test_name_dir);

	// Create a server pool
	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = String::from(test_name_dir);
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
	server_config.seed_addr = String::from(format!(
		"{}:{}",
		server_config.base_addr, server_config.p2p_server_port
	));

	// And create 4 more, then let them run for a while
	for i in 1..4 {
		// fudge in some slowdown
		server_config.miner_slowdown_in_millis = i * 2;
		pool.create_server(&mut server_config);
	}

	// pool.connect_all_peers();

	let _ = pool.run_all_servers();

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
		let s = grin::Server::new(config(10*n, test_name_dir, 0)).unwrap();
		servers.push(s);
		thread::sleep(time::Duration::from_millis(100));
	}

	// start mining
	servers[0].start_miner(miner_config());
	let original_height = servers[0].head().height;

	// monitor for a change of head on a different server and check whether
	// chain height has changed
	loop {
		let mut count = 0;
		for n in 0..5 {
			if servers[n].head().height > 3 {
				count += 1;
			}
		}
		if count == 5 {
			break;
		}
		thread::sleep(time::Duration::from_millis(100));
	}
	for n in 0..5 {
		servers[n].stop();
	}
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

	let s1 = grin::Server::new(config(1000, "grin-sync", 1000)).unwrap();
	// mine a few blocks on server 1
	s1.start_miner(miner_config());
	thread::sleep(time::Duration::from_secs(8));

	let mut conf = config(1001, "grin-sync", 1000);
	let s2 = grin::Server::new(conf).unwrap();
	while s2.head().height < 4 {
		thread::sleep(time::Duration::from_millis(100));
	}
	s1.stop();
	s2.stop();
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

	let s1 = grin::Server::new(config(2000, "grin-fast", 2000)).unwrap();
	// mine a few blocks on server 1
	s1.start_miner(miner_config());
	thread::sleep(time::Duration::from_secs(8));

	let mut conf = config(2001, "grin-fast", 2000);
	conf.archive_mode = Some(false);
	let s2 = grin::Server::new(conf).unwrap();
	while s2.head().height != s2.header_head().height || s2.head().height < 20 {
		thread::sleep(time::Duration::from_millis(1000));
	}
	s1.stop();
	s2.stop();
}

// #[test]
fn simulate_fast_sync_double() {
	util::init_test_logger();

	// we actually set the chain_type in the ServerConfig below
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	framework::clean_all_output("grin-double-fast1");
	framework::clean_all_output("grin-double-fast2");

	let s1 = grin::Server::new(config(3000, "grin-double-fast1", 3000)).unwrap();
	// mine a few blocks on server 1
	s1.start_miner(miner_config());
	thread::sleep(time::Duration::from_secs(8));

	{
		let mut conf = config(3001, "grin-double-fast2", 3000);
		conf.archive_mode = Some(false);
		let s2 = grin::Server::new(conf).unwrap();
		while s2.head().height != s2.header_head().height || s2.head().height < 20 {
			thread::sleep(time::Duration::from_millis(1000));
		}
		s2.stop();
	}
	// locks files don't seem to be cleaned properly until process exit
	std::fs::remove_file("target/tmp/grin-double-fast2/grin-sync-1001/chain/LOCK");
	std::fs::remove_file("target/tmp/grin-double-fast2/grin-sync-1001/peers/LOCK");
	thread::sleep(time::Duration::from_secs(20));

	let mut conf = config(3001, "grin-double-fast2", 3000);
	conf.archive_mode = Some(false);
	let s2 = grin::Server::new(conf).unwrap();
	while s2.head().height != s2.header_head().height || s2.head().height < 50 {
		thread::sleep(time::Duration::from_millis(1000));
	}
	s1.stop();
	s2.stop();
}

fn config(n: u16, test_name_dir: &str, seed_n: u16) -> grin::ServerConfig {
	grin::ServerConfig {
		api_http_addr: format!("127.0.0.1:{}", 20000 + n),
		db_root: format!("target/tmp/{}/grin-sync-{}", test_name_dir, n),
		p2p_config: p2p::P2PConfig {
			port: 10000 + n,
			..p2p::P2PConfig::default()
		},
		seeding_type: grin::Seeding::List,
		seeds: Some(vec![format!("127.0.0.1:{}", 10000 + seed_n)]),
		chain_type: core::global::ChainTypes::AutomatedTesting,
		archive_mode: Some(true),
		skip_sync_wait: Some(true),
		..Default::default()
	}
}

fn miner_config() -> pow::types::MinerConfig {
	let mut plugin_config = pow::types::CuckooMinerPluginConfig::default();
	let mut plugin_config_vec: Vec<pow::types::CuckooMinerPluginConfig> = Vec::new();
	plugin_config.type_filter = String::from("mean_cpu");
	plugin_config_vec.push(plugin_config);

	pow::types::MinerConfig {
		enable_mining: true,
		burn_reward: true,
		miner_async_mode: Some(false),
		miner_plugin_dir: None,
		miner_plugin_config: Some(plugin_config_vec),
		..Default::default()
	}
}
