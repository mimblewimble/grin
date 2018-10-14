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

//! Grin server implementation, glues the different parts of the system (mostly
//! the peer-to-peer server, the blockchain and the transaction pool) and acts
//! as a facade.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::{thread, time};

use api;
use chain;
use common::adapters::{
	ChainToPoolAndNetAdapter, NetToChainAdapter, PoolToChainAdapter, PoolToNetAdapter,
};
use common::stats::{DiffBlock, DiffStats, PeerStats, ServerStateInfo, ServerStats};
use common::types::{Error, ServerConfig, StratumServerConfig, SyncState};
use core::core::hash::Hashed;
use core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use core::pow::Difficulty;
use core::{consensus, genesis, global, pow};
use grin::{dandelion_monitor, seed, sync};
use mining::stratumserver;
use mining::test_miner::Miner;
use p2p;
use pool;
use store;
use util::file::get_first_line;
use util::LOGGER;

/// Grin server holding internal structures.
pub struct Server {
	/// server config
	pub config: ServerConfig,
	/// handle to our network server
	pub p2p: Arc<p2p::Server>,
	/// data store access
	pub chain: Arc<chain::Chain>,
	/// in-memory transaction pool
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	/// Shared cache for verification results when
	/// verifying rangeproof and kernel signatures.
	verifier_cache: Arc<RwLock<VerifierCache>>,
	/// Whether we're currently syncing
	sync_state: Arc<SyncState>,
	/// To be passed around to collect stats and info
	state_info: ServerStateInfo,
	/// Stop flag
	pub stop: Arc<AtomicBool>,
}

impl Server {
	/// Instantiates and starts a new server. Optionally takes a callback
	/// for the server to send an ARC copy of itself, to allow another process
	/// to poll info about the server status
	pub fn start<F>(config: ServerConfig, mut info_callback: F) -> Result<(), Error>
	where
		F: FnMut(Arc<Server>),
	{
		let mining_config = config.stratum_mining_config.clone();
		let enable_test_miner = config.run_test_miner;
		let test_miner_wallet_url = config.test_miner_wallet_url.clone();
		let serv = Arc::new(Server::new(config)?);

		if let Some(c) = mining_config {
			let enable_stratum_server = c.enable_stratum_server;
			if let Some(s) = enable_stratum_server {
				if s {
					{
						let mut stratum_stats = serv.state_info.stratum_stats.write().unwrap();
						stratum_stats.is_enabled = true;
					}
					serv.start_stratum_server(c.clone());
				}
			}
		}

		if let Some(s) = enable_test_miner {
			if s {
				serv.start_test_miner(test_miner_wallet_url, serv.stop.clone());
			}
		}

		info_callback(serv.clone());
		loop {
			thread::sleep(time::Duration::from_secs(1));
			if serv.stop.load(Ordering::Relaxed) {
				return Ok(());
			}
		}
	}

	/// Instantiates a new server associated with the provided future reactor.
	pub fn new(mut config: ServerConfig) -> Result<Server, Error> {
		// Defaults to None (optional) in config file.
		// This translates to false here.
		let archive_mode = match config.archive_mode {
			None => false,
			Some(b) => b,
		};

		// If archive mode is enabled then the flags should contains the FULL_HIST flag
		if archive_mode && !config
			.p2p_config
			.capabilities
			.contains(p2p::Capabilities::FULL_HIST)
		{
			config
				.p2p_config
				.capabilities
				.insert(p2p::Capabilities::FULL_HIST);
		}

		let stop = Arc::new(AtomicBool::new(false));

		// Shared cache for verification results.
		// We cache rangeproof verification and kernel signature verification.
		let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));

		let pool_adapter = Arc::new(PoolToChainAdapter::new());
		let pool_net_adapter = Arc::new(PoolToNetAdapter::new());
		let tx_pool = Arc::new(RwLock::new(pool::TransactionPool::new(
			config.pool_config.clone(),
			pool_adapter.clone(),
			verifier_cache.clone(),
			pool_net_adapter.clone(),
		)));

		let sync_state = Arc::new(SyncState::new());

		let chain_adapter = Arc::new(ChainToPoolAndNetAdapter::new(
			sync_state.clone(),
			tx_pool.clone(),
		));

		let genesis = match config.chain_type {
			global::ChainTypes::Testnet1 => genesis::genesis_testnet1(),
			global::ChainTypes::Testnet2 => genesis::genesis_testnet2(),
			global::ChainTypes::Testnet3 => genesis::genesis_testnet3(),
			global::ChainTypes::AutomatedTesting => genesis::genesis_dev(),
			global::ChainTypes::UserTesting => genesis::genesis_dev(),
			global::ChainTypes::Mainnet => genesis::genesis_testnet2(), //TODO: Fix, obviously
		};

		info!(LOGGER, "Starting server, genesis block: {}", genesis.hash());

		let db_env = Arc::new(store::new_env(config.db_root.clone()));
		let shared_chain = Arc::new(chain::Chain::init(
			config.db_root.clone(),
			db_env,
			chain_adapter.clone(),
			genesis.clone(),
			pow::verify_size,
			verifier_cache.clone(),
			archive_mode,
		)?);

		pool_adapter.set_chain(shared_chain.clone());

		let awaiting_peers = Arc::new(AtomicBool::new(false));

		let net_adapter = Arc::new(NetToChainAdapter::new(
			sync_state.clone(),
			archive_mode,
			shared_chain.clone(),
			tx_pool.clone(),
			verifier_cache.clone(),
			config.clone(),
		));

		let block_1_hash = match shared_chain.get_header_by_height(1) {
			Ok(header) => Some(header.hash()),
			Err(_) => None,
		};

		let peer_db_env = Arc::new(store::new_named_env(config.db_root.clone(), "peer".into()));
		let p2p_server = Arc::new(p2p::Server::new(
			peer_db_env,
			config.p2p_config.capabilities,
			config.p2p_config.clone(),
			net_adapter.clone(),
			genesis.hash(),
			stop.clone(),
			archive_mode,
			block_1_hash,
		)?);
		chain_adapter.init(p2p_server.peers.clone());
		pool_net_adapter.init(p2p_server.peers.clone());
		net_adapter.init(p2p_server.peers.clone());

		if config.p2p_config.seeding_type.clone() != p2p::Seeding::Programmatic {
			let seeder = match config.p2p_config.seeding_type.clone() {
				p2p::Seeding::None => {
					warn!(
						LOGGER,
						"No seed configured, will stay solo until connected to"
					);
					seed::predefined_seeds(vec![])
				}
				p2p::Seeding::List => {
					seed::predefined_seeds(config.p2p_config.seeds.as_mut().unwrap().clone())
				}
				p2p::Seeding::DNSSeed => seed::dns_seeds(),
				_ => unreachable!(),
			};

			let peers_preferred = match config.p2p_config.peers_preferred.clone() {
				Some(peers_preferred) => seed::preferred_peers(peers_preferred),
				None => None,
			};

			seed::connect_and_monitor(
				p2p_server.clone(),
				config.p2p_config.capabilities,
				config.dandelion_config.clone(),
				seeder,
				peers_preferred,
				stop.clone(),
			);
		}

		// Defaults to None (optional) in config file.
		// This translates to false here so we do not skip by default.
		let skip_sync_wait = match config.skip_sync_wait {
			None => false,
			Some(b) => b,
		};

		sync::run_sync(
			sync_state.clone(),
			awaiting_peers.clone(),
			p2p_server.peers.clone(),
			shared_chain.clone(),
			skip_sync_wait,
			archive_mode,
			stop.clone(),
		);

		let p2p_inner = p2p_server.clone();
		let _ = thread::Builder::new()
			.name("p2p-server".to_string())
			.spawn(move || p2p_inner.listen());

		info!(LOGGER, "Starting rest apis at: {}", &config.api_http_addr);
		let api_secret = get_first_line(config.api_secret_path.clone());
		api::start_rest_apis(
			config.api_http_addr.clone(),
			shared_chain.clone(),
			tx_pool.clone(),
			p2p_server.peers.clone(),
			api_secret,
			None,
		);

		info!(
			LOGGER,
			"Starting dandelion monitor: {}", &config.api_http_addr
		);
		dandelion_monitor::monitor_transactions(
			config.dandelion_config.clone(),
			tx_pool.clone(),
			verifier_cache.clone(),
			stop.clone(),
		);

		warn!(LOGGER, "Grin server started.");
		Ok(Server {
			config,
			p2p: p2p_server,
			chain: shared_chain,
			tx_pool,
			verifier_cache,
			sync_state,
			state_info: ServerStateInfo {
				awaiting_peers: awaiting_peers,
				..Default::default()
			},
			stop,
		})
	}

	/// Asks the server to connect to a peer at the provided network address.
	pub fn connect_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		self.p2p.connect(&addr)?;
		Ok(())
	}

	/// Ping all peers, mostly useful for tests to have connected peers share
	/// their heights
	pub fn ping_peers(&self) -> Result<(), Error> {
		let head = self.chain.head()?;
		self.p2p.peers.check_all(head.total_difficulty, head.height);
		Ok(())
	}

	/// Number of peers
	pub fn peer_count(&self) -> u32 {
		self.p2p.peers.peer_count()
	}

	/// Start a minimal "stratum" mining service on a separate thread
	pub fn start_stratum_server(&self, config: StratumServerConfig) {
		let cuckoo_size = global::min_sizeshift();
		let proof_size = global::proofsize();
		let sync_state = self.sync_state.clone();

		let mut stratum_server = stratumserver::StratumServer::new(
			config.clone(),
			self.chain.clone(),
			self.tx_pool.clone(),
			self.verifier_cache.clone(),
		);
		let stratum_stats = self.state_info.stratum_stats.clone();
		let _ = thread::Builder::new()
			.name("stratum_server".to_string())
			.spawn(move || {
				stratum_server.run_loop(stratum_stats, cuckoo_size as u32, proof_size, sync_state);
			});
	}

	/// Start mining for blocks internally on a separate thread. Relies on
	/// internal miner, and should only be used for automated testing. Burns
	/// reward if wallet_listener_url is 'None'
	pub fn start_test_miner(&self, wallet_listener_url: Option<String>, stop: Arc<AtomicBool>) {
		info!(LOGGER, "start_test_miner - start",);
		let sync_state = self.sync_state.clone();
		let config_wallet_url = match wallet_listener_url.clone() {
			Some(u) => u,
			None => String::from("http://127.0.0.1:13415"),
		};

		let config = StratumServerConfig {
			attempt_time_per_block: 60,
			burn_reward: false,
			enable_stratum_server: None,
			stratum_server_addr: None,
			wallet_listener_url: config_wallet_url,
			minimum_share_difficulty: 1,
		};

		let mut miner = Miner::new(
			config.clone(),
			self.chain.clone(),
			self.tx_pool.clone(),
			self.verifier_cache.clone(),
			stop,
		);
		miner.set_debug_output_id(format!("Port {}", self.config.p2p_config.port));
		let _ = thread::Builder::new()
			.name("test_miner".to_string())
			.spawn(move || {
				// TODO push this down in the run loop so miner gets paused anytime we
				// decide to sync again
				let secs_5 = time::Duration::from_secs(5);
				while sync_state.is_syncing() {
					thread::sleep(secs_5);
				}
				miner.run_loop(wallet_listener_url);
			});
	}

	/// The chain head
	pub fn head(&self) -> chain::Tip {
		self.chain.head().unwrap()
	}

	/// The head of the block header chain
	pub fn header_head(&self) -> chain::Tip {
		self.chain.header_head().unwrap()
	}

	/// Returns a set of stats about this server. This and the ServerStats
	/// structure
	/// can be updated over time to include any information needed by tests or
	/// other
	/// consumers
	pub fn get_server_stats(&self) -> Result<ServerStats, Error> {
		let stratum_stats = self.state_info.stratum_stats.read().unwrap().clone();
		let awaiting_peers = self.state_info.awaiting_peers.load(Ordering::Relaxed);

		// Fill out stats on our current difficulty calculation
		// TODO: check the overhead of calculating this again isn't too much
		// could return it from next_difficulty, but would rather keep consensus
		// code clean. This may be handy for testing but not really needed
		// for release
		let diff_stats = {
			let last_blocks: Vec<Result<(u64, Difficulty), consensus::TargetError>> =
				global::difficulty_data_to_vector(self.chain.difficulty_iter())
					.into_iter()
					.skip(consensus::MEDIAN_TIME_WINDOW as usize)
					.take(consensus::DIFFICULTY_ADJUST_WINDOW as usize)
					.collect();

			let mut last_time = last_blocks[0].clone().unwrap().0;
			let tip_height = self.chain.head().unwrap().height as i64;
			let earliest_block_height = tip_height as i64 - last_blocks.len() as i64;

			let mut i = 1;

			let diff_entries: Vec<DiffBlock> = last_blocks
				.iter()
				.skip(1)
				.map(|n| {
					let (time, diff) = n.clone().unwrap();
					let dur = time - last_time;
					let height = earliest_block_height + i + 1;
					i += 1;
					last_time = time;
					DiffBlock {
						block_number: height,
						difficulty: diff.to_num(),
						time: time,
						duration: dur,
					}
				}).collect();

			let block_time_sum = diff_entries.iter().fold(0, |sum, t| sum + t.duration);
			let block_diff_sum = diff_entries.iter().fold(0, |sum, d| sum + d.difficulty);
			DiffStats {
				height: tip_height as u64,
				last_blocks: diff_entries,
				average_block_time: block_time_sum / (consensus::DIFFICULTY_ADJUST_WINDOW - 1),
				average_difficulty: block_diff_sum / (consensus::DIFFICULTY_ADJUST_WINDOW - 1),
				window_size: consensus::DIFFICULTY_ADJUST_WINDOW,
			}
		};

		let peer_stats = self
			.p2p
			.peers
			.connected_peers()
			.into_iter()
			.map(|p| PeerStats::from_peer(&p))
			.collect();
		Ok(ServerStats {
			peer_count: self.peer_count(),
			head: self.head(),
			header_head: self.header_head(),
			sync_status: self.sync_state.status(),
			awaiting_peers: awaiting_peers,
			stratum_stats: stratum_stats,
			peer_stats: peer_stats,
			diff_stats: diff_stats,
		})
	}

	/// Stop the server.
	pub fn stop(&self) {
		self.p2p.stop();
		self.stop.store(true, Ordering::Relaxed);
	}

	/// Stops the test miner without stopping the p2p layer
	pub fn stop_test_miner(&self, stop: Arc<AtomicBool>) {
		stop.store(true, Ordering::Relaxed);
		info!(LOGGER, "stop_test_miner - stop",);
	}
}
