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

#[macro_use]
extern crate router;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_grin as grin;
extern crate grin_p2p as p2p;
extern crate grin_pow as pow;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate futures;
extern crate tokio_core;
extern crate tokio_timer;

mod framework;

use std::thread;
use std::time;
use std::default::Default;

use futures::{Async, Future, Poll};
use futures::task::current;
use tokio_core::reactor;
use tokio_timer::Timer;

use core::consensus;
use core::global;
use core::global::ChainTypes;
use wallet::WalletConfig;

use framework::{LocalServerContainer, LocalServerContainerConfig, LocalServerContainerPool,
                LocalServerContainerPoolConfig};

/// Testing the frameworks by starting a fresh server, creating a genesis
/// Block and mining into a wallet for a bit
#[test]
fn basic_genesis_mine() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "genesis_mine";
	framework::clean_all_output(test_name_dir);

	// Create a server pool
	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = String::from(test_name_dir);
	pool_config.run_length_in_seconds = 5;

	pool_config.base_api_port = 30000;
	pool_config.base_p2p_port = 31000;
	pool_config.base_wallet_port = 32000;

	let mut pool = LocalServerContainerPool::new(pool_config);

	// Create a server to add into the pool
	let mut server_config = LocalServerContainerConfig::default();
	server_config.start_miner = true;
	server_config.start_wallet = true;

	pool.create_server(&mut server_config);
	pool.run_all_servers();
}

/// Creates 5 servers, first being a seed and check that through peer address
/// messages they all end up connected.
#[test]
fn simulate_seeding() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "simulate_seeding";
	framework::clean_all_output(test_name_dir);

	// Create a server pool
	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = String::from(test_name_dir);
	pool_config.run_length_in_seconds = 30;

	// have to select different ports because of tests being run in parallel
	pool_config.base_api_port = 30020;
	pool_config.base_p2p_port = 31020;
	pool_config.base_wallet_port = 32020;

	let mut pool = LocalServerContainerPool::new(pool_config);

	// Create a first seed server to add into the pool
	let mut server_config = LocalServerContainerConfig::default();
	// server_config.start_miner = true;
	server_config.start_wallet = true;
	server_config.is_seeding = true;

	pool.create_server(&mut server_config);

	// point next servers at first seed
	server_config.is_seeding = false;
	server_config.seed_addr = String::from(format!(
		"{}:{}",
		server_config.base_addr,
		server_config.p2p_server_port
	));

	for _ in 0..4 {
		pool.create_server(&mut server_config);
	}

	pool.connect_all_peers();

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
	// have to select different ports because of tests being run in parallel
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
		server_config.base_addr,
		server_config.p2p_server_port
	));

	// And create 4 more, then let them run for a while
	for i in 1..4 {
		// fudge in some slowdown
		server_config.miner_slowdown_in_millis = i * 2;
		pool.create_server(&mut server_config);
	}

	pool.connect_all_peers();

	let _ = pool.run_all_servers();

	// Check mining difficulty here?, though I'd think it's more valuable
 // to simply output it. Can at least see the evolution of the difficulty target
 // in the debug log output for now
}

// TODO: Convert these tests to newer framework format
/// Create a network of 5 servers and mine a block, verifying that the block
/// gets propagated to all.
#[test]
fn a_simulate_block_propagation() {
	util::init_test_logger();
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "grin-prop";
	framework::clean_all_output(test_name_dir);
	let mut evtlp = reactor::Core::new().unwrap();
	let handle = evtlp.handle();

	let mut plugin_config = pow::types::CuckooMinerPluginConfig::default();
	let mut plugin_config_vec: Vec<pow::types::CuckooMinerPluginConfig> = Vec::new();
	plugin_config.type_filter = String::from("mean_cpu");
	plugin_config_vec.push(plugin_config);

	let miner_config = pow::types::MinerConfig {
		enable_mining: true,
		burn_reward: true,
		use_cuckoo_miner: false,
		cuckoo_miner_async_mode: None,
		cuckoo_miner_plugin_dir: Some(String::from("../target/debug/deps")),
		cuckoo_miner_plugin_config: Some(plugin_config_vec),
		..Default::default()
	};

	// instantiates 5 servers on different ports
	let mut servers = vec![];
	for n in 0..5 {
		let s = grin::Server::future(
			grin::ServerConfig {
				api_http_addr: format!("127.0.0.1:{}", 19000 + n),
				db_root: format!("target/{}/grin-prop-{}", test_name_dir, n),
				p2p_config: Some(p2p::P2PConfig {
					port: 18000 + n,
					..p2p::P2PConfig::default()
				}),
				seeding_type: grin::Seeding::List,
				seeds: Some(vec!["127.0.0.1:18000".to_string()]),
				..Default::default()
			},
			&handle,
		).unwrap();
		servers.push(s);
	}

	// start mining
	servers[0].start_miner(miner_config);
	let original_height = servers[0].head().height;

	// monitor for a change of head on a different server and check whether
	// chain height has changed
	evtlp.run(change(&servers[4]).and_then(|tip| {
		assert!(tip.height == original_height + 1);
		Ok(())
	}));
}

/// Creates 2 different disconnected servers, mine a few blocks on one, connect
/// them and check that the 2nd gets all the blocks
#[test]
fn simulate_full_sync() {
	util::init_test_logger();
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let test_name_dir = "grin-sync";
	framework::clean_all_output(test_name_dir);

	let mut evtlp = reactor::Core::new().unwrap();
	let handle = evtlp.handle();

	let mut plugin_config = pow::types::CuckooMinerPluginConfig::default();
	let mut plugin_config_vec: Vec<pow::types::CuckooMinerPluginConfig> = Vec::new();
	plugin_config.type_filter = String::from("mean_cpu");
	plugin_config_vec.push(plugin_config);

	let miner_config = pow::types::MinerConfig {
		enable_mining: true,
		burn_reward: true,
		use_cuckoo_miner: false,
		cuckoo_miner_async_mode: Some(false),
		cuckoo_miner_plugin_dir: Some(String::from("../target/debug/deps")),
		cuckoo_miner_plugin_config: Some(plugin_config_vec),
		..Default::default()
	};

	// instantiates 2 servers on different ports
	let mut servers = vec![];
	for n in 0..2 {
		let mut config = grin::ServerConfig {
			api_http_addr: format!("127.0.0.1:{}", 19000 + n),
			db_root: format!("target/{}/grin-sync-{}", test_name_dir, n),
			p2p_config: Some(p2p::P2PConfig {
				port: 11000 + n,
				..p2p::P2PConfig::default()
			}),
			seeding_type: grin::Seeding::List,
			seeds: Some(vec!["127.0.0.1:11000".to_string()]),
			..Default::default()
		};
		let s = grin::Server::future(config, &handle).unwrap();
		servers.push(s);
	}

	// mine a few blocks on server 1
	servers[0].start_miner(miner_config);
	thread::sleep(time::Duration::from_secs(5));

	// 2 should get blocks
	evtlp.run(change(&servers[1]));
}

// Builds the change future, monitoring for a change of head on the provided
// server
fn change<'a>(s: &'a grin::Server) -> HeadChange<'a> {
	let start_head = s.head();
	HeadChange {
		server: s,
		original: start_head,
	}
}

/// Future that monitors when a server has had its head updated. Current
/// implementation isn't optimized, only use for tests.
struct HeadChange<'a> {
	server: &'a grin::Server,
	original: chain::Tip,
}

impl<'a> Future for HeadChange<'a> {
	type Item = chain::Tip;
	type Error = ();

	fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
		let new_head = self.server.head();
		if new_head.last_block_h != self.original.last_block_h {
			Ok(Async::Ready(new_head))
		} else {
			// egregious polling, asking the task to schedule us every iteration
			current().notify();
			Ok(Async::NotReady)
		}
	}
}
