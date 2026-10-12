// Copyright 2021 The Grin Developers
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

//! Multi-node simulation tests (mining, seeding, propagation, sync).
//!
//! Ported from the pre-wallet-split `servers/tests` suite (grin#2957).
//! Wallet-dependent scenarios stay in grin-wallet.

#[macro_use]
extern crate log;

mod common;

use crate::common::{
	clean_all_output, config, init_chain, settle, start_mining_server, start_server,
	stop_all_servers, LocalServerContainerConfig, LocalServerContainerPool,
	LocalServerContainerPoolConfig,
};
use grin_core::core::hash::Hashed;
use grin_util as util;
use grin_util::StopState;
use std::sync::Arc;
use std::{thread, time};

/// Single node mines for a short time then shuts down cleanly.
#[test]
fn basic_genesis_mine() {
	util::init_test_logger();
	init_chain();

	let test_name_dir = "genesis_mine";
	clean_all_output(test_name_dir);

	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = String::from(test_name_dir);
	pool_config.run_length_in_seconds = 10;
	pool_config.base_api_port = 30000;
	pool_config.base_p2p_port = 31000;

	let mut pool = LocalServerContainerPool::new(pool_config);

	let mut server_config = LocalServerContainerConfig::default();
	server_config.start_miner = true;
	server_config.is_seeding = false;

	pool.create_server(&mut server_config);
	let servers = pool.run_all_servers();
	// Allow a few mining iterations.
	thread::sleep(time::Duration::from_secs(8));
	assert!(servers[0].head().unwrap().height >= 1);
	stop_all_servers(servers);
	settle();
}

/// One seed plus four peers; all should end up connected via peer exchange.
#[test]
fn simulate_seeding() {
	util::init_test_logger();
	init_chain();

	let test_name_dir = "simulate_seeding";
	clean_all_output(test_name_dir);

	let mut pool_config = LocalServerContainerPoolConfig::default();
	pool_config.base_name = test_name_dir.to_string();
	pool_config.run_length_in_seconds = 30;
	pool_config.base_api_port = 30020;
	pool_config.base_p2p_port = 31020;

	let mut pool = LocalServerContainerPool::new(pool_config);

	let mut server_config = LocalServerContainerConfig::default();
	server_config.start_miner = false;
	server_config.is_seeding = true;

	pool.create_server(&mut server_config);

	// Seed fully up before remaining servers.
	thread::sleep(time::Duration::from_millis(1_000));

	server_config.is_seeding = false;
	server_config.seed_addr = format!(
		"{}:{}",
		server_config.base_addr, server_config.p2p_server_port
	);

	for _ in 0..4 {
		pool.create_server(&mut server_config);
	}

	let servers = pool.run_all_servers();
	thread::sleep(time::Duration::from_secs(8));

	// Seed should see all four peers connected.
	let seed = servers
		.iter()
		.find(|s| s.config.p2p_config.port == 31020)
		.expect("seed server");
	assert_eq!(seed.peer_count(), 4);

	stop_all_servers(servers);
	settle();
}

/// Five connected nodes; mine on one and verify block height propagates.
#[test]
fn simulate_block_propagation() {
	util::init_test_logger();
	init_chain();

	let test_name_dir = "grin-prop";
	clean_all_output(test_name_dir);

	let mut servers = vec![];
	for n in 0..5 {
		let s = start_server(config(10 * n, test_name_dir, 0));
		servers.push(s);
		thread::sleep(time::Duration::from_millis(100));
	}

	let stop = Arc::new(StopState::new());
	servers[0].start_test_miner(None, stop.clone());

	let mut success = false;
	let mut time_spent = 0;
	loop {
		let mut count = 0;
		for n in 0..5 {
			if servers[n].head().unwrap().height > 3 {
				count += 1;
			}
		}
		if count == 5 {
			success = true;
			break;
		}
		thread::sleep(time::Duration::from_millis(1_000));
		time_spent += 1;
		if time_spent >= 45 {
			info!("simulate_block_propagation - fail on timeout");
			break;
		}
		if time_spent == 12 {
			servers[0].stop_test_miner(stop.clone());
		}
	}

	stop_all_servers(servers);
	assert!(success, "all 5 nodes should reach height > 3");
	settle();
}

/// Mine on s1, start s2 seeded from s1, verify s2 syncs headers/blocks.
#[test]
fn simulate_full_sync() {
	util::init_test_logger();
	init_chain();

	let test_name_dir = "grin-sync";
	clean_all_output(test_name_dir);

	let (s1, miner_stop) = start_mining_server(config(1000, test_name_dir, 1000));
	thread::sleep(time::Duration::from_secs(10));
	s1.stop_test_miner(miner_stop);
	// Let the miner thread exit so the tip is stable.
	thread::sleep(time::Duration::from_secs(2));

	let s1_header = s1.chain.head_header().unwrap();
	info!(
		"simulate_full_sync - s1 header head: {} at {}",
		s1_header.hash(),
		s1_header.height
	);
	assert!(
		s1_header.height >= 1,
		"s1 should have mined at least one block"
	);

	let s2 = start_server(config(1001, test_name_dir, 1000));

	let mut time_spent = 0;
	let target = s1_header.height;
	while s2.head().unwrap().height < target {
		thread::sleep(time::Duration::from_millis(1_000));
		time_spent += 1;
		if time_spent >= 45 {
			info!(
				"sync fail. s2.height: {}, s1.height: {}",
				s2.head().unwrap().height,
				s1.head().unwrap().height
			);
			break;
		}
	}

	// Compare live tips (both archive nodes; full block body sync).
	let s1_tip = s1.chain.head_header().unwrap();
	let s2_tip = s2.chain.head_header().unwrap();
	assert_eq!(s1_tip.height, s2_tip.height);
	assert_eq!(s1_tip.hash(), s2_tip.hash());

	s1.stop();
	s2.stop();
	settle();
}

/// Mine past the state-sync threshold on archive s1; pruned s2 should catch up.
///
/// On current main, non-archive peers use PIBD for state sync. This test asserts
/// header + body catch-up when both peers stay in range for body sync after
/// headers are aligned (s2 starts as archive so we exercise long-chain body
/// sync without depending on live PIBD segment serving in CI). A dedicated
/// PIBD multi-node test can be added once segment serving is hardened for
/// AutomatedTesting.
#[test]
fn simulate_long_chain_sync() {
	util::init_test_logger();
	init_chain();

	let test_name_dir = "grin-long-sync";
	clean_all_output(test_name_dir);

	let (s1, miner_stop) = start_mining_server(config(2000, test_name_dir, 2000));

	while s1.head().unwrap().height < 25 {
		thread::sleep(time::Duration::from_millis(1_000));
	}
	s1.stop_test_miner(miner_stop);
	thread::sleep(time::Duration::from_secs(2));

	// Second archive node seeds from s1 and body-syncs the full history.
	let s2 = start_server(config(2001, test_name_dir, 2000));

	let s1_header = s1.chain.head_header().unwrap();

	let mut total_wait = 0;
	while s2.head().unwrap().height < s1_header.height {
		thread::sleep(time::Duration::from_millis(1_000));
		total_wait += 1;
		if total_wait >= 90 {
			error!(
				"simulate_long_chain_sync timeout! s2 height: {}, s1 height: {}",
				s2.head().unwrap().height,
				s1_header.height,
			);
			break;
		}
	}

	let s1_tip = s1.chain.head_header().unwrap();
	let s2_tip = s2.chain.head_header().unwrap();
	assert_eq!(s1_tip.height, s2_tip.height);
	assert_eq!(s1_tip.hash(), s2_tip.hash());
	assert!(s1_tip.height >= 25);

	s1.stop();
	s2.stop();
	settle();
}
