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
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time;

use adapters::*;
use api;
use chain;
use core::{genesis, global};
use miner;
use p2p;
use pool;
use seed;
use sync;
use types::*;
use pow;
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
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	currently_syncing: Arc<AtomicBool>,
	stop: Arc<AtomicBool>,
}

impl Server {
	/// Instantiates and starts a new server.
	pub fn start(config: ServerConfig) -> Result<(), Error> {
		let mut mining_config = config.mining_config.clone();
		let serv = Server::new(config)?;
		if mining_config.as_mut().unwrap().enable_mining {
			serv.start_miner(mining_config.unwrap());
		}

		loop {
			thread::sleep(time::Duration::from_secs(1));
			if serv.stop.load(Ordering::Relaxed) {
				return Ok(());
			}
		}
	}

	/// Instantiates a new server associated with the provided future reactor.
	pub fn new(mut config: ServerConfig) -> Result<Server, Error> {
		let stop = Arc::new(AtomicBool::new(false));

		let pool_adapter = Arc::new(PoolToChainAdapter::new());
		let pool_net_adapter = Arc::new(PoolToNetAdapter::new());
		let tx_pool = Arc::new(RwLock::new(pool::TransactionPool::new(
			config.pool_config.clone(),
			pool_adapter.clone(),
			pool_net_adapter.clone(),
		)));

		let chain_adapter = Arc::new(ChainToPoolAndNetAdapter::new(tx_pool.clone()));

		let genesis = match config.chain_type {
			global::ChainTypes::Testnet1 => genesis::genesis_testnet1(),
			global::ChainTypes::Testnet2 => genesis::genesis_testnet2(),
			_ => pow::mine_genesis_block(config.mining_config.clone())?,
		};
		info!(LOGGER, "Starting server, genesis block: {}", genesis.hash(),);

		let shared_chain = Arc::new(chain::Chain::init(
			config.db_root.clone(),
			chain_adapter.clone(),
			genesis.clone(),
			pow::verify_size,
		)?);

		pool_adapter.set_chain(Arc::downgrade(&shared_chain));

		let currently_syncing = Arc::new(AtomicBool::new(true));

		let net_adapter = Arc::new(NetToChainAdapter::new(
			currently_syncing.clone(),
			Arc::downgrade(&shared_chain),
			tx_pool.clone(),
		));

		let p2p_config = config.p2p_config.clone();
		let p2p_server = Arc::new(p2p::Server::new(
			config.db_root.clone(),
			config.capabilities,
			p2p_config,
			net_adapter.clone(),
			genesis.hash(),
			stop.clone(),
		)?);
		chain_adapter.init(Arc::downgrade(&p2p_server.peers));
		pool_net_adapter.init(Arc::downgrade(&p2p_server.peers));
		net_adapter.init(Arc::downgrade(&p2p_server.peers));

		if config.seeding_type.clone() != Seeding::Programmatic {
			let seeder = match config.seeding_type.clone() {
				Seeding::None => {
					warn!(
						LOGGER,
						"No seed configured, will stay solo until connected to"
					);
					seed::predefined_seeds(vec![])
				}
				Seeding::List => seed::predefined_seeds(config.seeds.as_mut().unwrap().clone()),
				Seeding::WebStatic => seed::web_seeds(),
				_ => unreachable!(),
			};
			seed::connect_and_monitor(
				p2p_server.clone(),
				config.capabilities,
				seeder,
				stop.clone(),
			);
		}

		// Defaults to None (optional) in config file.
		// This translates to false here.
		let archive_mode = match config.archive_mode {
			None => false,
			Some(b) => b,
		};

		// Defaults to None (optional) in config file.
		// This translates to false here so we do not skip by default.
		let skip_sync_wait = match config.skip_sync_wait {
			None => false,
			Some(b) => b,
		};

		sync::run_sync(
			currently_syncing.clone(),
			p2p_server.peers.clone(),
			shared_chain.clone(),
			skip_sync_wait,
			!archive_mode,
			stop.clone(),
		);

		let p2p_inner = p2p_server.clone();
		let _ = thread::Builder::new()
			.name("p2p-server".to_string())
			.spawn(move || p2p_inner.listen());

		info!(LOGGER, "Starting rest apis at: {}", &config.api_http_addr);

		api::start_rest_apis(
			config.api_http_addr.clone(),
			Arc::downgrade(&shared_chain),
			Arc::downgrade(&tx_pool),
			Arc::downgrade(&p2p_server.peers),
		);

		warn!(LOGGER, "Grin server started.");
		Ok(Server {
			config: config,
			p2p: p2p_server,
			chain: shared_chain,
			tx_pool: tx_pool,
			currently_syncing: currently_syncing,
			stop: stop,
		})
	}

	/// Asks the server to connect to a peer at the provided network address.
	pub fn connect_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		self.p2p.connect(&addr)?;
		Ok(())
	}

	/// Number of peers
	pub fn peer_count(&self) -> u32 {
		self.p2p.peers.peer_count()
	}

	/// Start mining for blocks on a separate thread. Uses toy miner by default,
	/// mostly for testing, but can also load a plugin from cuckoo-miner
	pub fn start_miner(&self, config: pow::types::MinerConfig) {
		let cuckoo_size = global::sizeshift();
		let proof_size = global::proofsize();
		let currently_syncing = self.currently_syncing.clone();

		let mut miner = miner::Miner::new(
			config.clone(),
			self.chain.clone(),
			self.tx_pool.clone(),
			self.stop.clone(),
		);
		miner.set_debug_output_id(format!("Port {}", self.config.p2p_config.port));
		let _ = thread::Builder::new()
			.name("miner".to_string())
			.spawn(move || {
				// TODO push this down in the run loop so miner gets paused anytime we
				// decide to sync again
				let secs_5 = time::Duration::from_secs(5);
				while currently_syncing.load(Ordering::Relaxed) {
					thread::sleep(secs_5);
				}
				miner.run_loop(config.clone(), cuckoo_size as u32, proof_size);
			});
	}

	/// The chain head
	pub fn head(&self) -> chain::Tip {
		self.chain.head().unwrap()
	}

	/// The head of the block header chain
	pub fn header_head(&self) -> chain::Tip {
		self.chain.get_header_head().unwrap()
	}

	/// Returns a set of stats about this server. This and the ServerStats
	/// structure
	/// can be updated over time to include any information needed by tests or
	/// other
	/// consumers
	pub fn get_server_stats(&self) -> Result<ServerStats, Error> {
		Ok(ServerStats {
			peer_count: self.peer_count(),
			head: self.head(),
		})
	}

	/// Stop the server.
	pub fn stop(&self) {
		self.p2p.stop();
		self.stop.store(true, Ordering::Relaxed);
	}
}
