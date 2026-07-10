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

//! Shared helpers for multi-node integration tests.
//!
//! Node-only: mining rewards are burned (no wallet dependency).

#![allow(dead_code)]

use futures::channel::oneshot;
use grin_core as core;
use grin_core::global::{self, ChainTypes};
use grin_p2p as p2p;
use grin_servers as servers;
use grin_util::StopState;
use p2p::msg::PeerAddrs;
use p2p::PeerAddr;
use std::default::Default;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::{fs, thread, time};

/// Configure AutomatedTesting for the test thread and all server worker threads.
pub fn init_chain() {
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	global::set_global_chain_type(ChainTypes::AutomatedTesting);
}

/// Remove leftover data from a previous run of this test.
pub fn clean_all_output(test_name_dir: &str) {
	let target_dir = format!("target/tmp/{}", test_name_dir);
	if let Err(e) = fs::remove_dir_all(&target_dir) {
		// Missing dir is fine on first run.
		if Path::new(&target_dir).exists() {
			println!(
				"can't remove output from previous test {}: {}, may be ok",
				target_dir, e
			);
		}
	}
}

/// Leak a oneshot channel pair required by `Server::new` for API shutdown.
pub fn leak_api_chan() -> &'static mut (oneshot::Sender<()>, oneshot::Receiver<()>) {
	Box::leak(Box::new(oneshot::channel::<()>()))
}

/// Parse `ip:port` into a `PeerAddr`.
pub fn peer_addr(addr: &str) -> PeerAddr {
	PeerAddr(addr.parse().expect("valid peer address"))
}

/// Build a `ServerConfig` for integration tests.
///
/// - `n` offsets ports so parallel tests do not collide
/// - `seed_n` is the peer index used when `seeding_type` is `List`
pub fn config(n: u16, test_name_dir: &str, seed_n: u16) -> servers::ServerConfig {
	let seed = SocketAddr::new(
		IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
		10000 + seed_n,
	);
	servers::ServerConfig {
		api_http_addr: format!("127.0.0.1:{}", 20000 + n),
		api_secret_path: None,
		foreign_api_secret_path: None,
		db_root: format!("target/tmp/{}/grin-sync-{}", test_name_dir, n),
		p2p_config: p2p::P2PConfig {
			host: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
			port: 10000 + n,
			seeding_type: p2p::Seeding::List,
			seeds: Some(PeerAddrs {
				peers: vec![PeerAddr(seed)],
			}),
			..p2p::P2PConfig::default()
		},
		chain_type: core::global::ChainTypes::AutomatedTesting,
		archive_mode: Some(true),
		skip_sync_wait: Some(true),
		run_tui: Some(false),
		run_test_miner: Some(false),
		stratum_mining_config: None,
		..Default::default()
	}
}

/// Stratum mining config suitable for tests (rewards burned).
pub fn stratum_config() -> servers::StratumServerConfig {
	servers::StratumServerConfig {
		enable_stratum_server: Some(true),
		stratum_server_addr: Some(String::from("127.0.0.1:13416")),
		attempt_time_per_block: 60,
		minimum_share_difficulty: 1,
		wallet_listener_url: String::from("http://127.0.0.1:13415"),
		burn_reward: true,
	}
}

/// Start a server with the given config.
pub fn start_server(cfg: servers::ServerConfig) -> servers::Server {
	servers::Server::new(cfg, None, None, leak_api_chan()).expect("server starts")
}

/// Start a server and attach the internal test miner (burns coinbase).
pub fn start_mining_server(cfg: servers::ServerConfig) -> (servers::Server, Arc<StopState>) {
	let server = start_server(cfg);
	let miner_stop = Arc::new(StopState::new());
	server.start_test_miner(None, miner_stop.clone());
	(server, miner_stop)
}

/// Stop a list of servers cleanly (consumes them).
pub fn stop_all_servers(servers: Vec<servers::Server>) {
	for s in servers {
		s.stop();
	}
}

/// Brief pause so ports and locks release between tests.
pub fn settle() {
	thread::sleep(time::Duration::from_millis(500));
}

/// Configuration for a single local server instance.
#[derive(Clone)]
pub struct LocalServerContainerConfig {
	pub name: String,
	pub base_addr: String,
	pub p2p_server_port: u16,
	pub api_server_port: u16,
	pub start_miner: bool,
	pub seed_addr: String,
	pub is_seeding: bool,
	pub peer_list: Vec<String>,
}

impl Default for LocalServerContainerConfig {
	fn default() -> LocalServerContainerConfig {
		LocalServerContainerConfig {
			name: String::from("test_host"),
			base_addr: String::from("127.0.0.1"),
			api_server_port: 13413,
			p2p_server_port: 13414,
			seed_addr: String::from(""),
			is_seeding: false,
			start_miner: false,
			peer_list: Vec::new(),
		}
	}
}

/// Builds and starts a single node (optionally mining).
pub struct LocalServerContainer {
	pub config: LocalServerContainerConfig,
	working_dir: String,
}

impl LocalServerContainer {
	pub fn new(config: LocalServerContainerConfig) -> LocalServerContainer {
		let working_dir = format!("target/tmp/{}", config.name);
		LocalServerContainer {
			config,
			working_dir,
		}
	}

	pub fn add_peer(&mut self, addr: String) {
		self.config.peer_list.push(addr);
	}

	/// Start the server (and optional test miner). Returns the running server.
	pub fn run_server(self) -> servers::Server {
		let api_addr = format!("{}:{}", self.config.base_addr, self.config.api_server_port);

		let mut seeding_type = p2p::Seeding::None;
		let mut seeds = PeerAddrs { peers: vec![] };

		if !self.config.seed_addr.is_empty() {
			seeding_type = p2p::Seeding::List;
			seeds.peers = vec![peer_addr(&self.config.seed_addr)];
		}

		let cfg = servers::ServerConfig {
			api_http_addr: api_addr,
			api_secret_path: None,
			foreign_api_secret_path: None,
			db_root: format!("{}/.grin", self.working_dir),
			p2p_config: p2p::P2PConfig {
				host: self
					.config
					.base_addr
					.parse()
					.unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
				port: self.config.p2p_server_port,
				seeds: if seeds.peers.is_empty() {
					None
				} else {
					Some(seeds)
				},
				seeding_type,
				..p2p::P2PConfig::default()
			},
			chain_type: core::global::ChainTypes::AutomatedTesting,
			skip_sync_wait: Some(true),
			archive_mode: Some(true),
			run_tui: Some(false),
			run_test_miner: Some(false),
			stratum_mining_config: None,
			..Default::default()
		};

		let s = start_server(cfg);

		if self.config.start_miner {
			println!(
				"starting test Miner on port {}",
				self.config.p2p_server_port
			);
			s.start_test_miner(None, s.stop_state.clone());
		}

		for p in &self.config.peer_list {
			println!("{} connecting to peer: {}", self.config.p2p_server_port, p);
			let _ = s.connect_peer(peer_addr(p));
		}

		s
	}
}

/// Pool configuration for multi-server tests.
pub struct LocalServerContainerPoolConfig {
	pub base_name: String,
	pub base_p2p_port: u16,
	pub base_api_port: u16,
	pub run_length_in_seconds: u64,
}

impl Default for LocalServerContainerPoolConfig {
	fn default() -> LocalServerContainerPoolConfig {
		LocalServerContainerPoolConfig {
			base_name: String::from("test_pool"),
			base_p2p_port: 10000,
			base_api_port: 11000,
			run_length_in_seconds: 30,
		}
	}
}

/// Convenience pool for starting several servers with consecutive ports.
pub struct LocalServerContainerPool {
	pub config: LocalServerContainerPoolConfig,
	server_containers: Vec<LocalServerContainer>,
	next_p2p_port: u16,
	next_api_port: u16,
	is_seeding: bool,
}

impl LocalServerContainerPool {
	pub fn new(config: LocalServerContainerPoolConfig) -> LocalServerContainerPool {
		LocalServerContainerPool {
			next_api_port: config.base_api_port,
			next_p2p_port: config.base_p2p_port,
			config,
			server_containers: Vec::new(),
			is_seeding: false,
		}
	}

	/// Add a server using the next free ports. Mutates `server_config` with assigned values.
	pub fn create_server(&mut self, server_config: &mut LocalServerContainerConfig) {
		server_config.p2p_server_port = self.next_p2p_port;
		server_config.api_server_port = self.next_api_port;
		server_config.name = format!(
			"{}/{}-{}",
			self.config.base_name, self.config.base_name, server_config.p2p_server_port
		);

		self.next_p2p_port += 1;
		self.next_api_port += 1;

		if server_config.is_seeding {
			self.is_seeding = true;
		}

		self.server_containers
			.push(LocalServerContainer::new(server_config.clone()));
	}

	/// Start all servers, returning owned `Server` instances.
	pub fn run_all_servers(self) -> Vec<servers::Server> {
		let mut handles = vec![];
		let return_containers = Arc::new(std::sync::Mutex::new(Vec::new()));
		let is_seeding = self.is_seeding;

		for s in self.server_containers {
			let return_container_ref = return_containers.clone();
			let handle = thread::spawn(move || {
				if is_seeding && !s.config.is_seeding {
					// Give the seed a head start.
					thread::sleep(time::Duration::from_millis(2000));
				}
				let server_ref = s.run_server();
				return_container_ref.lock().unwrap().push(server_ref);
			});
			// RocksDB concurrent create can fail without a short gap.
			thread::sleep(time::Duration::from_millis(500));
			handles.push(handle);
		}

		for handle in handles {
			handle.join().expect("server thread");
		}

		// Keep run_length for API compatibility with old tests that waited this long.
		let _ = self.config.run_length_in_seconds;

		let mut guard = return_containers.lock().unwrap();
		std::mem::take(&mut *guard)
	}
}
