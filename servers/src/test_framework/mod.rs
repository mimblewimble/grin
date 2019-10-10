// Copyright 2019 The Grin Developers
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

use crate::api;
use crate::common::types::ServerConfig;
use crate::core;
use crate::grin::server::Server;
use crate::p2p::{self, PeerAddr};
use crate::util;
use easy_jsonrpc_mw::Handler;
use std::sync::Arc;
use std::{fs, thread, time};

#[allow(dead_code)]
pub fn run_doctest(request: serde_json::Value) -> Result<Option<serde_json::Value>, String> {
	util::init_test_logger();
	let api_server_one_dir = "api_server_one";
	let mut server_config = TestServerConfig::default();
	server_config.name = String::from(api_server_one_dir);
	server_config.p2p_server_port = 40000;
	server_config.api_server_port = 40001;
	server_config.start_miner = true;

	clean_test_dir(api_server_one_dir);

	String::from(format!("http://{}:{}", server_config.base_addr, 50002));
	let mut server_one = TestServer::new(server_config.clone()).unwrap();

	// Spawn server and let it run for a bit
	let _ = server_one.start();
	thread::sleep(time::Duration::from_millis(10000));

	//Wait for chain to build
	let server = server_one.server.unwrap();
	let api_node = api::Node::new(
		Arc::downgrade(&server.chain),
		Arc::downgrade(&server.tx_pool),
		Arc::downgrade(&server.p2p.peers),
		Arc::downgrade(&server.sync_state),
	);
	//node_api.doctest_mode = true;
	let node_api = &api_node as &dyn api::NodeRpc;
	let res = node_api.handle_request(request).as_option();
	clean_test_dir(api_server_one_dir);
	Ok(res)
}

/// TestServerConfig
#[derive(Debug, Clone)]
pub struct TestServerConfig {
	// user friendly name for the server, also denotes what dir
	// the data files will appear in
	pub name: String,

	// Base IP address
	pub base_addr: String,

	// Port the server (p2p) is running on
	pub p2p_server_port: u16,

	// Port the API server is running on
	pub api_server_port: u16,

	// Whether we're going to mine
	pub start_miner: bool,

	// time in millis by which to artificially slow down the mining loop
	// in this container
	pub miner_slowdown_in_millis: u64,

	// address of a server to use as a seed
	pub seed_addr: String,

	// keep track of whether this server is supposed to be seeding
	pub is_seeding: bool,
}

/// Default server config
impl Default for TestServerConfig {
	fn default() -> TestServerConfig {
		let name = String::from("test_host");
		TestServerConfig {
			name: name,
			base_addr: String::from("127.0.0.1"),
			api_server_port: 13413,
			p2p_server_port: 13414,
			seed_addr: String::from(""),
			is_seeding: false,
			start_miner: false,
			miner_slowdown_in_millis: 0,
		}
	}
}

/// Errors that can be returned by TestServer
#[derive(Debug)]
pub enum Error {
	Internal(String),
	Argument(String),
	NotFound,
}

/// A top-level container to hold everything that might be running
/// on a server, i.e. server, wallet in send or receive mode
pub struct TestServer {
	// Configuration
	config: TestServerConfig,
	// the inner server
	server: Option<Arc<Server>>,

	// the list of peers to connect to
	peer_list: Vec<String>,
	// base directory for the server instance
	pub working_dir: String,
}

impl TestServer {
	/// Create a new local server container with defaults, with the given name
	/// all related files will be created in the directory
	/// target/tmp/{name}
	pub fn new(config: TestServerConfig) -> Result<TestServer, Error> {
		let working_dir = format!("target/tmp/{}", config.name);
		Ok(TestServer {
			server: None,
			config: config,
			peer_list: Vec::new(),
			working_dir: working_dir,
		})
	}

	pub fn start(&mut self) -> Result<(), Error> {
		let api_addr = format!("{}:{}", self.config.base_addr, self.config.api_server_port);

		let mut seeding_type = p2p::Seeding::None;
		let mut seeds = Vec::new();

		if self.config.seed_addr.len() > 0 {
			seeding_type = p2p::Seeding::List;
			seeds = vec![PeerAddr::from_ip(
				self.config.seed_addr.to_string().parse().unwrap(),
			)];
		}
		core::global::set_mining_mode(core::global::ChainTypes::AutomatedTesting);

		let s = Server::new(ServerConfig {
			api_http_addr: api_addr,
			api_secret_path: None,
			db_root: format!("{}/.grin", self.working_dir),
			p2p_config: p2p::P2PConfig {
				port: self.config.p2p_server_port,
				seeds: Some(seeds),
				seeding_type: seeding_type,
				..p2p::P2PConfig::default()
			},
			chain_type: core::global::ChainTypes::AutomatedTesting,
			skip_sync_wait: Some(true),
			stratum_mining_config: None,
			..Default::default()
		})
		.unwrap();

		if self.config.start_miner == true {
			println!(
				"starting test Miner on port {}",
				self.config.p2p_server_port
			);
			s.start_test_miner(None, s.stop_state.clone());
		}

		for p in &mut self.peer_list {
			println!("{} connecting to peer: {}", self.config.p2p_server_port, p);
			let _ = s.connect_peer(PeerAddr::from_ip(p.parse().unwrap()));
		}
		self.server = Some(Arc::new(s));
		Ok(())
	}
}

/// Just removes all results from previous runs
pub fn clean_test_dir(test_name_dir: &str) {
	let target_dir = format!("target/tmp/{}", test_name_dir);
	if let Err(e) = fs::remove_dir_all(target_dir) {
		println!("can't remove output from previous test :{}, may be ok", e);
	}
}
