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

use self::core::global::{self, ChainTypes};
use self::util::init_test_logger;
use self::util::Mutex;
use crate::framework::{LocalServerContainer, LocalServerContainerConfig};
use grin_api as api;
use grin_core as core;
use grin_p2p as p2p;
use grin_util as util;
use std::sync::Arc;
use std::{thread, time};

#[test]
fn simple_server_wallet() {
	init_test_logger();
	info!("starting simple_server_wallet");
	let _test_name_dir = "test_servers";
	core::global::set_mining_mode(core::global::ChainTypes::AutomatedTesting);

	// Run a separate coinbase wallet for coinbase transactions
	let coinbase_dir = "coinbase_wallet_api";
	framework::clean_all_output(coinbase_dir);
	let mut coinbase_config = LocalServerContainerConfig::default();
	coinbase_config.name = String::from(coinbase_dir);
	coinbase_config.wallet_validating_node_url = String::from("http://127.0.0.1:40001");
	coinbase_config.wallet_port = 50002;
	let coinbase_wallet = Arc::new(Mutex::new(
		LocalServerContainer::new(coinbase_config).unwrap(),
	));

	let _ = thread::spawn(move || {
		let mut w = coinbase_wallet.lock();
		w.run_wallet(0);
	});

	// Wait for the wallet to start
	thread::sleep(time::Duration::from_millis(1000));

	let api_server_one_dir = "api_server_one";
	framework::clean_all_output(api_server_one_dir);
	let mut server_config = LocalServerContainerConfig::default();
	server_config.name = String::from(api_server_one_dir);
	server_config.p2p_server_port = 40000;
	server_config.api_server_port = 40001;
	server_config.start_miner = true;
	server_config.start_wallet = false;
	server_config.coinbase_wallet_address =
		String::from(format!("http://{}:{}", server_config.base_addr, 50002));
	let mut server_one = LocalServerContainer::new(server_config.clone()).unwrap();

	// Spawn server and let it run for a bit
	let _ = thread::spawn(move || server_one.run_server(120));

	//Wait for chain to build
	thread::sleep(time::Duration::from_millis(5000));

	// Starting tests
	let base_addr = server_config.base_addr;
	let api_server_port = server_config.api_server_port;

	warn!("Testing chain handler");
	let tip = get_tip(&base_addr, api_server_port);
	assert!(tip.is_ok());

	warn!("Testing status handler");
	let status = get_status(&base_addr, api_server_port);
	assert!(status.is_ok());

	// Be sure that at least a block is mined by Travis
	let mut current_tip = get_tip(&base_addr, api_server_port).unwrap();
	while current_tip.height == 0 {
		thread::sleep(time::Duration::from_millis(1000));
		current_tip = get_tip(&base_addr, api_server_port).unwrap();
	}

	warn!("Testing block handler");
	let last_block_by_height = get_block_by_height(&base_addr, api_server_port, current_tip.height);
	assert!(last_block_by_height.is_ok());
	let last_block_by_height_compact =
		get_block_by_height_compact(&base_addr, api_server_port, current_tip.height);
	assert!(last_block_by_height_compact.is_ok());

	let block_hash = current_tip.last_block_pushed;
	let last_block_by_hash = get_block_by_hash(&base_addr, api_server_port, &block_hash);
	assert!(last_block_by_hash.is_ok());
	let last_block_by_hash_compact =
		get_block_by_hash_compact(&base_addr, api_server_port, &block_hash);
	assert!(last_block_by_hash_compact.is_ok());

	warn!("Testing chain output handler");
	let start_height = 0;
	let end_height = current_tip.height;
	let outputs_by_height =
		get_outputs_by_height(&base_addr, api_server_port, start_height, end_height);
	assert!(outputs_by_height.is_ok());
	let ids = get_ids_from_block_outputs(outputs_by_height.unwrap());
	let outputs_by_ids1 = get_outputs_by_ids1(&base_addr, api_server_port, ids.clone());
	assert!(outputs_by_ids1.is_ok());
	let outputs_by_ids2 = get_outputs_by_ids2(&base_addr, api_server_port, ids.clone());
	assert!(outputs_by_ids2.is_ok());

	warn!("Testing txhashset handler");
	let roots = get_txhashset_roots(&base_addr, api_server_port);
	assert!(roots.is_ok());
	let last_10_outputs = get_txhashset_lastoutputs(&base_addr, api_server_port, 0);
	assert!(last_10_outputs.is_ok());
	let last_5_outputs = get_txhashset_lastoutputs(&base_addr, api_server_port, 5);
	assert!(last_5_outputs.is_ok());
	let last_10_rangeproofs = get_txhashset_lastrangeproofs(&base_addr, api_server_port, 0);
	assert!(last_10_rangeproofs.is_ok());
	let last_5_rangeproofs = get_txhashset_lastrangeproofs(&base_addr, api_server_port, 5);
	assert!(last_5_rangeproofs.is_ok());
	let last_10_kernels = get_txhashset_lastkernels(&base_addr, api_server_port, 0);
	assert!(last_10_kernels.is_ok());
	let last_5_kernels = get_txhashset_lastkernels(&base_addr, api_server_port, 5);
	assert!(last_5_kernels.is_ok());

	//let some more mining happen, make sure nothing pukes
	thread::sleep(time::Duration::from_millis(5000));
}

/// Creates 2 servers and test P2P API
#[test]
fn test_p2p() {
	init_test_logger();
	info!("starting test_p2p");
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let _test_name_dir = "test_servers";

	// Spawn server and let it run for a bit
	let server_one_dir = "p2p_server_one";
	framework::clean_all_output(server_one_dir);
	let mut server_config_one = LocalServerContainerConfig::default();
	server_config_one.name = String::from(server_one_dir);
	server_config_one.p2p_server_port = 40002;
	server_config_one.api_server_port = 40003;
	server_config_one.start_miner = false;
	server_config_one.start_wallet = false;
	server_config_one.is_seeding = true;
	let mut server_one = LocalServerContainer::new(server_config_one.clone()).unwrap();
	let _ = thread::spawn(move || server_one.run_server(120));

	thread::sleep(time::Duration::from_millis(1000));

	// Spawn server and let it run for a bit
	let server_two_dir = "p2p_server_two";
	framework::clean_all_output(server_two_dir);
	let mut server_config_two = LocalServerContainerConfig::default();
	server_config_two.name = String::from(server_two_dir);
	server_config_two.p2p_server_port = 40004;
	server_config_two.api_server_port = 40005;
	server_config_two.start_miner = false;
	server_config_two.start_wallet = false;
	server_config_two.is_seeding = false;
	let mut server_two = LocalServerContainer::new(server_config_two.clone()).unwrap();
	server_two.add_peer(format!(
		"{}:{}",
		server_config_one.base_addr, server_config_one.p2p_server_port
	));
	let _ = thread::spawn(move || server_two.run_server(120));

	// Let them do the handshake
	thread::sleep(time::Duration::from_millis(2000));

	// Starting tests
	warn!("Starting P2P Tests");
	let base_addr = server_config_one.base_addr;
	let api_server_port = server_config_one.api_server_port;

	// Check that peer all is also working
	let mut peers_all = get_all_peers(&base_addr, api_server_port);
	assert!(peers_all.is_ok());
	let pall = peers_all.unwrap();
	assert_eq!(pall.len(), 2);

	// Check that when we get peer connected the peer is here
	let peers_connected = get_connected_peers(&base_addr, api_server_port);
	assert!(peers_connected.is_ok());
	let pc = peers_connected.unwrap();
	assert_eq!(pc.len(), 1);

	// Check that the peer status is Healthy
	let addr = format!(
		"{}:{}",
		server_config_two.base_addr, server_config_two.p2p_server_port
	);
	let peer = get_peer(&base_addr, api_server_port, &addr);
	assert!(peer.is_ok());
	assert_eq!(peer.unwrap().flags, p2p::State::Healthy);

	// Ban the peer
	let ban_result = ban_peer(&base_addr, api_server_port, &addr);
	assert!(ban_result.is_ok());
	thread::sleep(time::Duration::from_millis(2000));

	// Check its status is banned with get peer
	let peer = get_peer(&base_addr, api_server_port, &addr);
	assert!(peer.is_ok());
	assert_eq!(peer.unwrap().flags, p2p::State::Banned);

	// Check from peer all
	peers_all = get_all_peers(&base_addr, api_server_port);
	assert!(peers_all.is_ok());
	assert_eq!(peers_all.unwrap().len(), 2);

	// Unban
	let unban_result = unban_peer(&base_addr, api_server_port, &addr);
	assert!(unban_result.is_ok());

	// Check from peer connected
	let peers_connected = get_connected_peers(&base_addr, api_server_port);
	assert!(peers_connected.is_ok());
	assert_eq!(peers_connected.unwrap().len(), 0);

	// Check its status is healthy with get peer
	let peer = get_peer(&base_addr, api_server_port, &addr);
	assert!(peer.is_ok());
	assert_eq!(peer.unwrap().flags, p2p::State::Healthy);
}

// Tip handler function
fn get_tip(base_addr: &String, api_server_port: u16) -> Result<api::Tip, Error> {
	let url = format!("http://{}:{}/v1/chain", base_addr, api_server_port);
	api::client::get::<api::Tip>(url.as_str(), None).map_err(|e| Error::API(e))
}

// Status handler function
fn get_status(base_addr: &String, api_server_port: u16) -> Result<api::Status, Error> {
	let url = format!("http://{}:{}/v1/status", base_addr, api_server_port);
	api::client::get::<api::Status>(url.as_str(), None).map_err(|e| Error::API(e))
}

// Block handler functions
fn get_block_by_height(
	base_addr: &String,
	api_server_port: u16,
	height: u64,
) -> Result<api::BlockPrintable, Error> {
	let url = format!(
		"http://{}:{}/v1/blocks/{}",
		base_addr, api_server_port, height
	);
	api::client::get::<api::BlockPrintable>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_block_by_height_compact(
	base_addr: &String,
	api_server_port: u16,
	height: u64,
) -> Result<api::CompactBlockPrintable, Error> {
	let url = format!(
		"http://{}:{}/v1/blocks/{}?compact",
		base_addr, api_server_port, height
	);
	api::client::get::<api::CompactBlockPrintable>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_block_by_hash(
	base_addr: &String,
	api_server_port: u16,
	block_hash: &String,
) -> Result<api::BlockPrintable, Error> {
	let url = format!(
		"http://{}:{}/v1/blocks/{}",
		base_addr, api_server_port, block_hash
	);
	api::client::get::<api::BlockPrintable>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_block_by_hash_compact(
	base_addr: &String,
	api_server_port: u16,
	block_hash: &String,
) -> Result<api::CompactBlockPrintable, Error> {
	let url = format!(
		"http://{}:{}/v1/blocks/{}?compact",
		base_addr, api_server_port, block_hash
	);
	api::client::get::<api::CompactBlockPrintable>(url.as_str(), None).map_err(|e| Error::API(e))
}

// Chain output handler functions
fn get_outputs_by_ids1(
	base_addr: &String,
	api_server_port: u16,
	ids: Vec<String>,
) -> Result<Vec<api::Output>, Error> {
	let url = format!(
		"http://{}:{}/v1/chain/outputs/byids?id={}",
		base_addr,
		api_server_port,
		ids.join(",")
	);
	api::client::get::<Vec<api::Output>>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_outputs_by_ids2(
	base_addr: &String,
	api_server_port: u16,
	ids: Vec<String>,
) -> Result<Vec<api::Output>, Error> {
	let mut ids_string: String = String::from("");
	for id in ids {
		ids_string = ids_string + "?id=" + &id;
	}
	let ids_string = String::from(&ids_string[1..ids_string.len()]);
	let url = format!(
		"http://{}:{}/v1/chain/outputs/byids?{}",
		base_addr, api_server_port, ids_string
	);
	api::client::get::<Vec<api::Output>>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_outputs_by_height(
	base_addr: &String,
	api_server_port: u16,
	start_height: u64,
	end_height: u64,
) -> Result<Vec<api::BlockOutputs>, Error> {
	let url = format!(
		"http://{}:{}/v1/chain/outputs/byheight?start_height={}&end_height={}",
		base_addr, api_server_port, start_height, end_height
	);
	api::client::get::<Vec<api::BlockOutputs>>(url.as_str(), None).map_err(|e| Error::API(e))
}

// TxHashSet handler functions
fn get_txhashset_roots(base_addr: &String, api_server_port: u16) -> Result<api::TxHashSet, Error> {
	let url = format!(
		"http://{}:{}/v1/txhashset/roots",
		base_addr, api_server_port
	);
	api::client::get::<api::TxHashSet>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_txhashset_lastoutputs(
	base_addr: &String,
	api_server_port: u16,
	n: u64,
) -> Result<Vec<api::TxHashSetNode>, Error> {
	let url: String;
	if n == 0 {
		url = format!(
			"http://{}:{}/v1/txhashset/lastoutputs",
			base_addr, api_server_port
		);
	} else {
		url = format!(
			"http://{}:{}/v1/txhashset/lastoutputs?n={}",
			base_addr, api_server_port, n
		);
	}
	api::client::get::<Vec<api::TxHashSetNode>>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_txhashset_lastrangeproofs(
	base_addr: &String,
	api_server_port: u16,
	n: u64,
) -> Result<Vec<api::TxHashSetNode>, Error> {
	let url: String;
	if n == 0 {
		url = format!(
			"http://{}:{}/v1/txhashset/lastrangeproofs",
			base_addr, api_server_port
		);
	} else {
		url = format!(
			"http://{}:{}/v1/txhashset/lastrangeproofs?n={}",
			base_addr, api_server_port, n
		);
	}
	api::client::get::<Vec<api::TxHashSetNode>>(url.as_str(), None).map_err(|e| Error::API(e))
}

fn get_txhashset_lastkernels(
	base_addr: &String,
	api_server_port: u16,
	n: u64,
) -> Result<Vec<api::TxHashSetNode>, Error> {
	let url: String;
	if n == 0 {
		url = format!(
			"http://{}:{}/v1/txhashset/lastkernels",
			base_addr, api_server_port
		);
	} else {
		url = format!(
			"http://{}:{}/v1/txhashset/lastkernels?n={}",
			base_addr, api_server_port, n
		);
	}
	api::client::get::<Vec<api::TxHashSetNode>>(url.as_str(), None).map_err(|e| Error::API(e))
}

// Helper function to get a vec of commitment output ids from a vec of block
// outputs
fn get_ids_from_block_outputs(block_outputs: Vec<api::BlockOutputs>) -> Vec<String> {
	let mut ids: Vec<String> = Vec::new();
	for block_output in block_outputs {
		let outputs = &block_output.outputs;
		for output in outputs {
			ids.push(util::to_hex(output.clone().commit.0.to_vec()));
		}
	}
	ids.into_iter().take(100).collect()
}

pub fn ban_peer(base_addr: &String, api_server_port: u16, peer_addr: &String) -> Result<(), Error> {
	let url = format!(
		"http://{}:{}/v1/peers/{}/ban",
		base_addr, api_server_port, peer_addr
	);
	api::client::post_no_ret(url.as_str(), None, &"").map_err(|e| Error::API(e))
}

pub fn unban_peer(
	base_addr: &String,
	api_server_port: u16,
	peer_addr: &String,
) -> Result<(), Error> {
	let url = format!(
		"http://{}:{}/v1/peers/{}/unban",
		base_addr, api_server_port, peer_addr
	);
	api::client::post_no_ret(url.as_str(), None, &"").map_err(|e| Error::API(e))
}

pub fn get_peer(
	base_addr: &String,
	api_server_port: u16,
	peer_addr: &String,
) -> Result<p2p::PeerData, Error> {
	let url = format!(
		"http://{}:{}/v1/peers/{}",
		base_addr, api_server_port, peer_addr
	);
	api::client::get::<p2p::PeerData>(url.as_str(), None).map_err(|e| Error::API(e))
}

pub fn get_connected_peers(
	base_addr: &String,
	api_server_port: u16,
) -> Result<Vec<p2p::types::PeerInfoDisplay>, Error> {
	let url = format!(
		"http://{}:{}/v1/peers/connected",
		base_addr, api_server_port
	);
	api::client::get::<Vec<p2p::types::PeerInfoDisplay>>(url.as_str(), None)
		.map_err(|e| Error::API(e))
}

pub fn get_all_peers(
	base_addr: &String,
	api_server_port: u16,
) -> Result<Vec<p2p::PeerData>, Error> {
	let url = format!("http://{}:{}/v1/peers/all", base_addr, api_server_port);
	api::client::get::<Vec<p2p::PeerData>>(url.as_str(), None).map_err(|e| Error::API(e))
}

/// Error type wrapping underlying module errors.
#[derive(Debug)]
pub enum Error {
	/// Error originating from HTTP API calls.
	API(api::Error),
}
