// Copyright 2017 The Grin Developers
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
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_pow as pow;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate blake2_rfc as blake2;
extern crate futures;
extern crate futures_cpupool;
extern crate tokio_core;
extern crate tokio_timer;

use std::thread;
use std::time;
use std::default::Default;
use std::fs;
use std::sync::{Arc, Mutex};

use tokio_core::reactor;
use tokio_timer::Timer;

use util::secp::Secp256k1;
// TODO - why does this need self here? Missing something somewhere.
use self::keychain::Keychain;
use wallet::WalletConfig;

/// Just removes all results from previous runs
pub fn clean_all_output(test_name_dir: &str) {
	let target_dir = format!("target/test_servers/{}", test_name_dir);
	let result = fs::remove_dir_all(target_dir);
	if let Err(e) = result {
		println!("{}", e);
	}
}

/// Errors that can be returned by LocalServerContainer
#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
	Internal(String),
	Argument(String),
	NotFound,
}

/// All-in-one server configuration struct, for convenience
///
#[derive(Clone)]
pub struct LocalServerContainerConfig {
	// user friendly name for the server, also denotes what dir
	// the data files will appear in
	pub name: String,

	// Base IP address
	pub base_addr: String,

	// Port the server (p2p) is running on
	pub p2p_server_port: u16,

	// Port the API server is running on
	pub api_server_port: u16,

	// Port the wallet server is running on
	pub wallet_port: u16,

	// Whether we're going to mine
	pub start_miner: bool,

	// time in millis by which to artifically slow down the mining loop
	// in this container
	pub miner_slowdown_in_millis: u64,

	// Whether we're going to run a wallet as well,
	// can use same server instance as a validating node for convenience
	pub start_wallet: bool,

	// address of a server to use as a seed
	pub seed_addr: String,

	// keep track of whether this server is supposed to be seeding
	pub is_seeding: bool,

	// Whether to burn mining rewards
	pub burn_mining_rewards: bool,

	// full address to send coinbase rewards to
	pub coinbase_wallet_address: String,

	// When running a wallet, the address to check inputs and send
	// finalised transactions to,
	pub wallet_validating_node_url: String,
}

/// Default server config
impl Default for LocalServerContainerConfig {
	fn default() -> LocalServerContainerConfig {
		LocalServerContainerConfig {
			name: String::from("test_host"),
			base_addr: String::from("127.0.0.1"),
			api_server_port: 13413,
			p2p_server_port: 13414,
			wallet_port: 13415,
			seed_addr: String::from(""),
			is_seeding: false,
			start_miner: false,
			start_wallet: false,
			burn_mining_rewards: false,
			coinbase_wallet_address: String::from(""),
			wallet_validating_node_url: String::from(""),
			miner_slowdown_in_millis: 0,
		}
	}
}


/// A top-level container to hold everything that might be running
/// on a server, i.e. server, wallet in send or receive mode

pub struct LocalServerContainer {
	// Configuration
	config: LocalServerContainerConfig,

	// Structure of references to the
	// internal server data
	pub p2p_server_stats: Option<grin::ServerStats>,

	// The API server instance
	api_server: Option<api::ApiServer>,

	// whether the server is running
	pub server_is_running: bool,

	// Whether the server is mining
	pub server_is_mining: bool,

	// Whether the server is also running a wallet
	// Not used if running wallet without server
	pub wallet_is_running: bool,

	// the list of peers to connect to
	pub peer_list: Vec<String>,

	// base directory for the server instance
	working_dir: String,
}

impl LocalServerContainer {
	/// Create a new local server container with defaults, with the given name
	/// all related files will be created in the directory
	/// target/test_servers/{name}

	pub fn new(config: LocalServerContainerConfig) -> Result<LocalServerContainer, Error> {
		let working_dir = format!("target/test_servers/{}", config.name);
		Ok(
			(LocalServerContainer {
				config: config,
				p2p_server_stats: None,
				api_server: None,
				server_is_running: false,
				server_is_mining: false,
				wallet_is_running: false,
				working_dir: working_dir,
				peer_list: Vec::new(),
			}),
		)
	}

	pub fn run_server(&mut self, duration_in_seconds: u64) -> grin::ServerStats {
		let mut event_loop = reactor::Core::new().unwrap();

		let api_addr = format!("{}:{}", self.config.base_addr, self.config.api_server_port);

		let mut seeding_type = grin::Seeding::None;
		let mut seeds = Vec::new();

		if self.config.seed_addr.len() > 0 {
			seeding_type = grin::Seeding::List;
			seeds = vec![self.config.seed_addr.to_string()];
		}

		let s = grin::Server::future(
			grin::ServerConfig {
				api_http_addr: api_addr,
				db_root: format!("{}/.grin", self.working_dir),
				p2p_config: Some(p2p::P2PConfig {
					port: self.config.p2p_server_port,
					..p2p::P2PConfig::default()
				}),
				seeds: Some(seeds),
				seeding_type: seeding_type,
				..Default::default()
			},
			&event_loop.handle(),
		).unwrap();

		self.p2p_server_stats = Some(s.get_server_stats().unwrap());

		if self.config.start_wallet == true {
			self.run_wallet(duration_in_seconds + 5);
			// give a second to start wallet before continuing
			thread::sleep(time::Duration::from_millis(1000));
		}

		let mut plugin_config = pow::types::CuckooMinerPluginConfig::default();
		let mut plugin_config_vec: Vec<pow::types::CuckooMinerPluginConfig> = Vec::new();
		plugin_config.type_filter = String::from("mean_cpu");
		plugin_config_vec.push(plugin_config);

		let miner_config = pow::types::MinerConfig {
			enable_mining: self.config.start_miner,
			burn_reward: self.config.burn_mining_rewards,
			use_cuckoo_miner: false,
			cuckoo_miner_async_mode: Some(false),
			cuckoo_miner_plugin_dir: Some(String::from("../target/debug/deps")),
			cuckoo_miner_plugin_config: Some(plugin_config_vec),
			wallet_receiver_url: self.config.coinbase_wallet_address.clone(),
			slow_down_in_millis: Some(self.config.miner_slowdown_in_millis.clone()),
			..Default::default()
		};

		if self.config.start_miner == true {
			println!("starting Miner on port {}", self.config.p2p_server_port);
			s.start_miner(miner_config);
		}

		for p in &mut self.peer_list {
			println!("{} connecting to peer: {}", self.config.p2p_server_port, p);
			s.connect_peer(p.parse().unwrap()).unwrap();
		}

		let timeout = Timer::default().sleep(time::Duration::from_secs(duration_in_seconds));

		event_loop.run(timeout).unwrap();

		if self.wallet_is_running {
			self.stop_wallet();
		}

		s.get_server_stats().unwrap()
	}

	/// Starts a wallet daemon to receive and returns the
	/// listening server url

	pub fn run_wallet(&mut self, _duration_in_seconds: u64) {
		// URL on which to start the wallet listener (i.e. api server)
		let url = format!("{}:{}", self.config.base_addr, self.config.wallet_port);

		// Just use the name of the server for a seed for now
		let seed = format!("{}", self.config.name);

		let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());

		// TODO - just use from_random_seed here?
		let keychain =
			Keychain::from_seed(seed.as_bytes()).expect("Error initializing keychain from seed");

		println!(
			"Starting the Grin wallet receiving daemon on {} ",
			self.config.wallet_port
		);

		let mut wallet_config = WalletConfig::default();

		wallet_config.api_listen_port = format!("{}", self.config.wallet_port);
		wallet_config.check_node_api_http_addr = self.config.wallet_validating_node_url.clone();
		wallet_config.data_file_dir = self.working_dir.clone();

		let receive_tx_handler = wallet::WalletReceiver {
			config: wallet_config.clone(),
			keychain: keychain.clone(),
		};
		let router = router!(
			receive_tx: get "/receive/transaction" => receive_tx_handler,
		);

		let mut api_server = api::ApiServer::new("/v1".to_string());
		api_server.register_handler(router);
		api_server.start(url).unwrap_or_else(|e| {
			println!("Failed to start Grin wallet receiver: {}.", e);
		});

		self.api_server = Some(api_server);
		self.wallet_is_running = true;
	}

	/// Stops the running wallet server

	pub fn stop_wallet(&mut self) {
		let mut api_server = self.api_server.as_mut().unwrap();
		api_server.stop();
	}

	/// Adds a peer to this server to connect to upon running

	pub fn add_peer(&mut self, addr: String) {
		self.peer_list.push(addr);
	}
}

/// Configuration values for container pool

pub struct LocalServerContainerPoolConfig {
	// Base name to append to all the servers in this pool
	pub base_name: String,

	// Base http address for all of the servers in this pool
	pub base_http_addr: String,

	// Base port server for all of the servers in this pool
	// Increment the number by 1 for each new server
	pub base_p2p_port: u16,

	// Base api port for all of the servers in this pool
	// Increment this number by 1 for each new server
	pub base_api_port: u16,

	// Base wallet port for this server
	//
	pub base_wallet_port: u16,

	// How long the servers in the pool are going to run
	pub run_length_in_seconds: u64,
}

/// Default server config
///
impl Default for LocalServerContainerPoolConfig {
	fn default() -> LocalServerContainerPoolConfig {
		LocalServerContainerPoolConfig {
			base_name: String::from("test_pool"),
			base_http_addr: String::from("127.0.0.1"),
			base_p2p_port: 10000,
			base_api_port: 11000,
			base_wallet_port: 12000,
			run_length_in_seconds: 30,
		}
	}
}

/// A convenience pool for running many servers simultaneously
/// without necessarily having to configure each one manually

pub struct LocalServerContainerPool {
	// configuration
	pub config: LocalServerContainerPoolConfig,

	// keep ahold of all the created servers thread-safely
	server_containers: Vec<LocalServerContainer>,

	// Keep track of what the last ports a server was opened on
	next_p2p_port: u16,

	next_api_port: u16,

	next_wallet_port: u16,

	// keep track of whether a seed exists, and pause a bit if so
	is_seeding: bool,
}

impl LocalServerContainerPool {
	pub fn new(config: LocalServerContainerPoolConfig) -> LocalServerContainerPool {
		(LocalServerContainerPool {
			next_api_port: config.base_api_port,
			next_p2p_port: config.base_p2p_port,
			next_wallet_port: config.base_wallet_port,
			config: config,
			server_containers: Vec::new(),
			is_seeding: false,
		})
	}

	/// adds a single server on the next available port
	/// overriding passed-in values as necessary. Config object is an OUT value
	/// with
	/// ports/addresses filled in
	///

	pub fn create_server(&mut self, server_config: &mut LocalServerContainerConfig) {
		// If we're calling it this way, need to override these
		server_config.p2p_server_port = self.next_p2p_port;
		server_config.api_server_port = self.next_api_port;
		server_config.wallet_port = self.next_wallet_port;

		server_config.name = String::from(format!(
			"{}/{}-{}",
			self.config.base_name,
			self.config.base_name,
			server_config.p2p_server_port
		));


		// Use self as coinbase wallet
		server_config.coinbase_wallet_address = String::from(format!(
			"http://{}:{}",
			server_config.base_addr,
			server_config.wallet_port
		));


		self.next_p2p_port += 1;
		self.next_api_port += 1;
		self.next_wallet_port += 1;

		if server_config.is_seeding {
			self.is_seeding = true;
		}

		let _server_address = format!(
			"{}:{}",
			server_config.base_addr,
			server_config.p2p_server_port
		);

		let server_container = LocalServerContainer::new(server_config.clone()).unwrap();
		// self.server_containers.push(server_arc);

		// Create a future that runs the server for however many seconds
  // collect them all and run them in the run_all_servers
		let _run_time = self.config.run_length_in_seconds;

		self.server_containers.push(server_container);
	}

	/// adds n servers, ready to run
	///
	///
	#[allow(dead_code)]
	pub fn create_servers(&mut self, number: u16) {
		for _ in 0..number {
			// self.create_server();
		}
	}

	/// runs all servers, and returns a vector of references to the servers
	/// once they've all been run
	///

	pub fn run_all_servers(self) -> Vec<grin::ServerStats> {
		let run_length = self.config.run_length_in_seconds;
		let mut handles = vec![];

		// return handles to all of the servers, wrapped in mutexes, handles, etc
		let return_containers = Arc::new(Mutex::new(Vec::new()));

		let is_seeding = self.is_seeding.clone();

		for mut s in self.server_containers {
			let return_container_ref = return_containers.clone();
			let handle = thread::spawn(move || {
				if is_seeding && !s.config.is_seeding {
					// there's a seed and we're not it, so hang around longer and give the seed
	 // a chance to start
					thread::sleep(time::Duration::from_millis(2000));
				}
				let server_ref = s.run_server(run_length);
				return_container_ref.lock().unwrap().push(server_ref);
			});
			// Not a big fan of sleeping hack here, but there appears to be a
   // concurrency issue when creating files in rocksdb that causes
   // failure if we don't pause a bit before starting the next server
			thread::sleep(time::Duration::from_millis(500));
			handles.push(handle);
		}

		for handle in handles {
			match handle.join() {
				Ok(_) => {}
				Err(e) => {
					println!("Error starting server thread: {:?}", e);
					panic!(e);
				}
			}
		}
		// return a much simplified version of the results
		let return_vec = return_containers.lock().unwrap();
		return_vec.clone()
	}

	pub fn connect_all_peers(&mut self) {
		/// just pull out all currently active servers, build a list,
		/// and feed into all servers
		let mut server_addresses: Vec<String> = Vec::new();
		for s in &self.server_containers {
			let server_address = format!("{}:{}", s.config.base_addr, s.config.p2p_server_port);
			server_addresses.push(server_address);
		}

		for a in server_addresses {
			for s in &mut self.server_containers {
				if format!("{}:{}", s.config.base_addr, s.config.p2p_server_port) != a {
					s.add_peer(a.clone());
				}
			}
		}
	}
}
