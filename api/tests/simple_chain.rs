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
use util::{init_test_logger, LOGGER};

#[test]
fn simple_server_wallet() {
	let test_name_dir = "test_servers";
	core::global::set_mining_mode(core::global::ChainTypes::AutomatedTesting);
	framework::clean_all_output(test_name_dir);
	let mut log_config = util::LoggingConfig::default();
	//log_config.stdout_log_level = util::LogLevel::Trace;
	log_config.stdout_log_level = util::LogLevel::Info;
	//init_logger(Some(log_config));
	init_test_logger();

	// Run a separate coinbase wallet for coinbase transactions
	let mut coinbase_config = LocalServerContainerConfig::default();
	coinbase_config.name = String::from("coinbase_wallet");
	coinbase_config.wallet_validating_node_url=String::from("http://127.0.0.1:30001");
	coinbase_config.wallet_port = 10002;
	let coinbase_wallet = Arc::new(Mutex::new(LocalServerContainer::new(coinbase_config).unwrap()));

	let _ = thread::spawn(move || {
		let mut w = coinbase_wallet.lock().unwrap();
		w.run_wallet(0);
	});

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
	let mut server_one = LocalServerContainer::new(server_config.clone()).unwrap();

	// Spawn server and let it run for a bit
	let _ = thread::spawn(move || server_one.run_server(120));

	//Wait for chain to build
	thread::sleep(time::Duration::from_millis(5000));

	// Starting tests
	let base_addr = server_config.base_addr;
	let api_server_port = server_config.api_server_port;

	warn!(LOGGER, "Testing chain handler");
	let tip = get_tip(&base_addr, api_server_port);
	assert!(tip.is_ok());

	warn!(LOGGER, "Testing status handler");
	let status = get_status(&base_addr, api_server_port);
	assert!(status.is_ok());

	warn!(LOGGER, "Testing block handler");
	let current_tip = tip.unwrap();
	let height = current_tip.height;
	let last_block_by_height = get_block_by_height(&base_addr, api_server_port, height);
	assert!(last_block_by_height.is_ok());
	let last_block_by_height_compact = get_block_by_height_compact(&base_addr, api_server_port, height);
	assert!(last_block_by_height_compact.is_ok());

	let block_hash = current_tip.last_block_pushed;
	let last_block_by_hash = get_block_by_hash(&base_addr, api_server_port, &block_hash);
	assert!(last_block_by_hash.is_ok());
	let last_block_by_hash_compact = get_block_by_hash_compact(&base_addr, api_server_port, &block_hash);
	assert!(last_block_by_hash_compact.is_ok());




	//let some more mining happen, make sure nothing pukes
	thread::sleep(time::Duration::from_millis(5000));
}


fn get_tip(base_addr: &String, api_server_port: u16) -> Result<api::Tip, Error> {
	let url = format!("http://{}:{}/v1/chain", base_addr, api_server_port);
	api::client::get::<api::Tip>(url.as_str()).map_err(|e| Error::API(e))
}

fn get_status(base_addr: &String, api_server_port: u16) -> Result<api::Status, Error> {
	let url = format!("http://{}:{}/v1/status", base_addr, api_server_port);
	api::client::get::<api::Status>(url.as_str()).map_err(|e| Error::API(e))
}

fn get_block_by_height(base_addr: &String, api_server_port: u16, height: u64) -> Result<api::BlockPrintable, Error> {
	let url = format!("http://{}:{}/v1/blocks/{}", base_addr, api_server_port, height);
	api::client::get::<api::BlockPrintable>(url.as_str()).map_err(|e| Error::API(e))
}

fn get_block_by_height_compact(base_addr: &String, api_server_port: u16, height: u64) -> Result<api::CompactBlockPrintable, Error> {
	let url = format!("http://{}:{}/v1/blocks/{}?compact", base_addr, api_server_port, height);
	api::client::get::<api::CompactBlockPrintable>(url.as_str()).map_err(|e| Error::API(e))
}

fn get_block_by_hash(base_addr: &String, api_server_port: u16, block_hash: &String) -> Result<api::BlockPrintable, Error> {
	let url = format!("http://{}:{}/v1/blocks/{}", base_addr, api_server_port, block_hash);
	api::client::get::<api::BlockPrintable>(url.as_str()).map_err(|e| Error::API(e))
}

fn get_block_by_hash_compact(base_addr: &String, api_server_port: u16, block_hash: &String) -> Result<api::CompactBlockPrintable, Error> {
	let url = format!("http://{}:{}/v1/blocks/{}?compact", base_addr, api_server_port, block_hash);
	api::client::get::<api::CompactBlockPrintable>(url.as_str()).map_err(|e| Error::API(e))
}

#[allow(unused)]
fn get_all_peers(base_addr: String, api_server_port: u16) -> Result<api::Status, Error> {
	let url = format!("http://{}:{}/v1/status", base_addr, api_server_port);
	api::client::get::<api::Status>(url.as_str()).map_err(|e| Error::API(e))
}

/// Error type wrapping underlying module errors.
#[derive(Debug)]
enum Error {
	/// Error originating from HTTP API calls.
	API(api::Error),
}
