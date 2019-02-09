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

use self::keychain::Keychain;
use self::util::Mutex;
use self::wallet::{HTTPNodeClient, HTTPWalletCommAdapter, LMDBBackend, WalletConfig};
use blake2_rfc as blake2;
use grin_api as api;
use grin_core as core;
use grin_keychain as keychain;
use grin_p2p as p2p;
use grin_servers as servers;
use grin_util as util;
use grin_wallet as wallet;
use std::default::Default;
use std::ops::Deref;
use std::sync::Arc;
use std::{fs, thread, time};

/// Just removes all results from previous runs
pub fn clean_all_output(test_name_dir: &str) {
	let target_dir = format!("target/tmp/{}", test_name_dir);
	if let Err(e) = fs::remove_dir_all(target_dir) {
		println!("can't remove output from previous test :{}, may be ok", e);
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

	// Port the wallet owner API is running on
	pub owner_port: u16,

	// Whether to include the foreign API endpoints in the owner API
	pub owner_api_include_foreign: bool,

	// Whether we're going to mine
	pub start_miner: bool,

	// time in millis by which to artificially slow down the mining loop
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
			owner_port: 13420,
			owner_api_include_foreign: false,
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

#[allow(dead_code)]
pub struct LocalServerContainer {
	// Configuration
	config: LocalServerContainerConfig,

	// Structure of references to the
	// internal server data
	pub p2p_server_stats: Option<servers::ServerStats>,

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
	pub working_dir: String,

	// Wallet configuration
	pub wallet_config: WalletConfig,
}

impl LocalServerContainer {
	/// Create a new local server container with defaults, with the given name
	/// all related files will be created in the directory
	/// target/tmp/{name}

	pub fn new(config: LocalServerContainerConfig) -> Result<LocalServerContainer, Error> {
		let working_dir = format!("target/tmp/{}", config.name);
		let mut wallet_config = WalletConfig::default();

		wallet_config.api_listen_port = config.wallet_port;
		wallet_config.check_node_api_http_addr = config.wallet_validating_node_url.clone();
		wallet_config.owner_api_include_foreign = Some(config.owner_api_include_foreign);
		wallet_config.data_file_dir = working_dir.clone();
		Ok(LocalServerContainer {
			config: config,
			p2p_server_stats: None,
			api_server: None,
			server_is_running: false,
			server_is_mining: false,
			wallet_is_running: false,
			working_dir: working_dir,
			peer_list: Vec::new(),
			wallet_config: wallet_config,
		})
	}

	pub fn run_server(&mut self, duration_in_seconds: u64) -> servers::Server {
		let api_addr = format!("{}:{}", self.config.base_addr, self.config.api_server_port);

		let mut seeding_type = p2p::Seeding::None;
		let mut seeds = Vec::new();

		if self.config.seed_addr.len() > 0 {
			seeding_type = p2p::Seeding::List;
			seeds = vec![self.config.seed_addr.to_string()];
		}

		let s = servers::Server::new(servers::ServerConfig {
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

		self.p2p_server_stats = Some(s.get_server_stats().unwrap());

		let mut wallet_url = None;

		if self.config.start_wallet == true {
			self.run_wallet(duration_in_seconds + 5);
			// give a second to start wallet before continuing
			thread::sleep(time::Duration::from_millis(1000));
			wallet_url = Some(format!(
				"http://{}:{}",
				self.config.base_addr, self.config.wallet_port
			));
		}

		if self.config.start_miner == true {
			println!(
				"starting test Miner on port {}",
				self.config.p2p_server_port
			);
			s.start_test_miner(wallet_url, s.stop_state.clone());
		}

		for p in &mut self.peer_list {
			println!("{} connecting to peer: {}", self.config.p2p_server_port, p);
			let _ = s.connect_peer(p.parse().unwrap());
		}

		if self.wallet_is_running {
			self.stop_wallet();
		}

		s
	}

	/// Make a wallet for use in test endpoints (run_wallet and run_owner).
	fn make_wallet_for_tests(
		&mut self,
	) -> Arc<Mutex<LMDBBackend<HTTPNodeClient, keychain::ExtKeychain>>> {
		// URL on which to start the wallet listener (i.e. api server)
		let _url = format!("{}:{}", self.config.base_addr, self.config.wallet_port);

		// Just use the name of the server for a seed for now
		let seed = format!("{}", self.config.name);

		let _seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());

		println!(
			"Starting the Grin wallet receiving daemon on {} ",
			self.config.wallet_port
		);

		self.wallet_config = WalletConfig::default();

		self.wallet_config.api_listen_port = self.config.wallet_port;
		self.wallet_config.check_node_api_http_addr =
			self.config.wallet_validating_node_url.clone();
		self.wallet_config.data_file_dir = self.working_dir.clone();
		self.wallet_config.owner_api_include_foreign = Some(self.config.owner_api_include_foreign);

		let _ = fs::create_dir_all(self.wallet_config.clone().data_file_dir);
		let r = wallet::WalletSeed::init_file(&self.wallet_config, 32, None, "");

		let client_n = HTTPNodeClient::new(&self.wallet_config.check_node_api_http_addr, None);

		if let Err(_e) = r {
			//panic!("Error initializing wallet seed: {}", e);
		}

		let wallet: LMDBBackend<HTTPNodeClient, keychain::ExtKeychain> =
			LMDBBackend::new(self.wallet_config.clone(), "", client_n).unwrap_or_else(|e| {
				panic!(
					"Error creating wallet: {:?} Config: {:?}",
					e, self.wallet_config
				)
			});

		Arc::new(Mutex::new(wallet))
	}

	/// Starts a wallet daemon to receive
	pub fn run_wallet(&mut self, _duration_in_mills: u64) {
		let wallet = self.make_wallet_for_tests();

		wallet::controller::foreign_listener(wallet, &self.wallet_config.api_listen_addr(), None)
			.unwrap_or_else(|e| {
				panic!(
					"Error creating wallet listener: {:?} Config: {:?}",
					e, self.wallet_config
				)
			});

		self.wallet_is_running = true;
	}

	/// Starts a wallet owner daemon
	#[allow(dead_code)]
	pub fn run_owner(&mut self) {
		let wallet = self.make_wallet_for_tests();

		// WalletConfig doesn't allow changing the owner API path, so we build
		// the path ourselves
		let owner_listen_addr = format!("127.0.0.1:{}", self.config.owner_port);

		wallet::controller::owner_listener(
			wallet,
			&owner_listen_addr,
			None,
			None,
			self.wallet_config.owner_api_include_foreign.clone(),
		)
		.unwrap_or_else(|e| {
			panic!(
				"Error creating wallet owner listener: {:?} Config: {:?}",
				e, self.wallet_config
			)
		});
	}

	#[allow(dead_code)]
	pub fn get_wallet_seed(config: &WalletConfig) -> wallet::WalletSeed {
		let _ = fs::create_dir_all(config.clone().data_file_dir);
		wallet::WalletSeed::init_file(config, 32, None, "").unwrap();
		let wallet_seed =
			wallet::WalletSeed::from_file(config, "").expect("Failed to read wallet seed file.");
		wallet_seed
	}

	#[allow(dead_code)]
	pub fn get_wallet_info(
		config: &WalletConfig,
		wallet_seed: &wallet::WalletSeed,
	) -> wallet::WalletInfo {
		let keychain: keychain::ExtKeychain = wallet_seed
			.derive_keychain(false)
			.expect("Failed to derive keychain from seed file and passphrase.");
		let client_n = HTTPNodeClient::new(&config.check_node_api_http_addr, None);
		let mut wallet = LMDBBackend::new(config.clone(), "", client_n)
			.unwrap_or_else(|e| panic!("Error creating wallet: {:?} Config: {:?}", e, config));
		wallet.keychain = Some(keychain);
		let parent_id = keychain::ExtKeychain::derive_key_id(2, 0, 0, 0, 0);
		let _ =
			wallet::libwallet::internal::updater::refresh_outputs(&mut wallet, &parent_id, false);
		wallet::libwallet::internal::updater::retrieve_info(&mut wallet, &parent_id, 1).unwrap()
	}

	#[allow(dead_code)]
	pub fn send_amount_to(
		config: &WalletConfig,
		amount: &str,
		minimum_confirmations: u64,
		selection_strategy: &str,
		dest: &str,
		_fluff: bool,
	) {
		let amount = core::core::amount_from_hr_string(amount)
			.expect("Could not parse amount as a number with optional decimal point.");

		let wallet_seed =
			wallet::WalletSeed::from_file(config, "").expect("Failed to read wallet seed file.");

		let keychain: keychain::ExtKeychain = wallet_seed
			.derive_keychain(false)
			.expect("Failed to derive keychain from seed file and passphrase.");

		let client_n = HTTPNodeClient::new(&config.check_node_api_http_addr, None);
		let client_w = HTTPWalletCommAdapter::new();

		let change_outputs = 1;

		let mut wallet = LMDBBackend::new(config.clone(), "", client_n)
			.unwrap_or_else(|e| panic!("Error creating wallet: {:?} Config: {:?}", e, config));
		wallet.keychain = Some(keychain);
		let _ = wallet::controller::owner_single_use(Arc::new(Mutex::new(wallet)), |api| {
			let (mut slate, lock_fn) = api.initiate_tx(
				None,
				amount,
				minimum_confirmations,
				change_outputs,
				selection_strategy == "all",
				None,
			)?;
			slate = client_w.send_tx_sync(dest, &slate)?;
			api.finalize_tx(&mut slate)?;
			api.tx_lock_outputs(&slate, lock_fn)?;
			println!(
				"Tx sent: {} grin to {} (strategy '{}')",
				core::core::amount_to_hr_string(amount, false),
				dest,
				selection_strategy,
			);
			Ok(())
		})
		.unwrap_or_else(|e| panic!("Error creating wallet: {:?} Config: {:?}", e, config));
	}

	/// Stops the running wallet server
	pub fn stop_wallet(&mut self) {
		println!("Stop wallet!");
		let api_server = self.api_server.as_mut().unwrap();
		api_server.stop();
	}

	/// Adds a peer to this server to connect to upon running

	#[allow(dead_code)]
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

	// Base wallet owner port for this server
	//
	pub base_owner_port: u16,

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
			base_owner_port: 13000,
			run_length_in_seconds: 30,
		}
	}
}

/// A convenience pool for running many servers simultaneously
/// without necessarily having to configure each one manually

#[allow(dead_code)]
pub struct LocalServerContainerPool {
	// configuration
	pub config: LocalServerContainerPoolConfig,

	// keep ahold of all the created servers thread-safely
	server_containers: Vec<LocalServerContainer>,

	// Keep track of what the last ports a server was opened on
	next_p2p_port: u16,

	next_api_port: u16,

	next_wallet_port: u16,

	next_owner_port: u16,

	// keep track of whether a seed exists, and pause a bit if so
	is_seeding: bool,
}

#[allow(dead_code)]
impl LocalServerContainerPool {
	pub fn new(config: LocalServerContainerPoolConfig) -> LocalServerContainerPool {
		(LocalServerContainerPool {
			next_api_port: config.base_api_port,
			next_p2p_port: config.base_p2p_port,
			next_wallet_port: config.base_wallet_port,
			next_owner_port: config.base_owner_port,
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

	#[allow(dead_code)]
	pub fn create_server(&mut self, server_config: &mut LocalServerContainerConfig) {
		// If we're calling it this way, need to override these
		server_config.p2p_server_port = self.next_p2p_port;
		server_config.api_server_port = self.next_api_port;
		server_config.wallet_port = self.next_wallet_port;
		server_config.owner_port = self.next_owner_port;

		server_config.name = String::from(format!(
			"{}/{}-{}",
			self.config.base_name, self.config.base_name, server_config.p2p_server_port
		));

		// Use self as coinbase wallet
		server_config.coinbase_wallet_address = String::from(format!(
			"http://{}:{}",
			server_config.base_addr, server_config.wallet_port
		));

		self.next_p2p_port += 1;
		self.next_api_port += 1;
		self.next_wallet_port += 1;
		self.next_owner_port += 1;

		if server_config.is_seeding {
			self.is_seeding = true;
		}

		let _server_address = format!(
			"{}:{}",
			server_config.base_addr, server_config.p2p_server_port
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

	#[allow(dead_code)]
	pub fn run_all_servers(self) -> Arc<Mutex<Vec<servers::Server>>> {
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
				return_container_ref.lock().push(server_ref);
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
		return_containers.clone()
	}

	#[allow(dead_code)]
	pub fn connect_all_peers(&mut self) {
		// just pull out all currently active servers, build a list,
		// and feed into all servers
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

#[allow(dead_code)]
pub fn stop_all_servers(servers: Arc<Mutex<Vec<servers::Server>>>) {
	let locked_servs = servers.lock();
	for s in locked_servs.deref() {
		s.stop();
	}
}

/// Create and return a ServerConfig
#[allow(dead_code)]
pub fn config(n: u16, test_name_dir: &str, seed_n: u16) -> servers::ServerConfig {
	servers::ServerConfig {
		api_http_addr: format!("127.0.0.1:{}", 20000 + n),
		api_secret_path: None,
		db_root: format!("target/tmp/{}/grin-sync-{}", test_name_dir, n),
		p2p_config: p2p::P2PConfig {
			port: 10000 + n,
			seeding_type: p2p::Seeding::List,
			seeds: Some(vec![format!("127.0.0.1:{}", 10000 + seed_n)]),
			..p2p::P2PConfig::default()
		},
		chain_type: core::global::ChainTypes::AutomatedTesting,
		archive_mode: Some(true),
		skip_sync_wait: Some(true),
		..Default::default()
	}
}

/// return stratum mining config
#[allow(dead_code)]
pub fn stratum_config() -> servers::common::types::StratumServerConfig {
	servers::common::types::StratumServerConfig {
		enable_stratum_server: Some(true),
		stratum_server_addr: Some(String::from("127.0.0.1:13416")),
		attempt_time_per_block: 60,
		minimum_share_difficulty: 1,
		wallet_listener_url: String::from("http://127.0.0.1:13415"),
		burn_reward: false,
	}
}
