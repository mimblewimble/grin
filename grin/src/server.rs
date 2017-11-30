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

//! Grin server implementation, glues the different parts of the system (mostly
//! the peer-to-peer server, the blockchain and the transaction pool) and acts
//! as a facade.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time;

use futures::{Future, Stream};
use tokio_core::reactor;
use tokio_timer::Timer;

use adapters::*;
use api;
use chain;
use core::{global, genesis};
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
	/// event handle
	evt_handle: reactor::Handle,
	/// handle to our network server
	p2p: Arc<p2p::Server>,
	/// data store access
	chain: Arc<chain::Chain>,
	/// in-memory transaction pool
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	net_adapter: Arc<NetToChainAdapter>,
}

impl Server {
	/// Instantiates and starts a new server.
	pub fn start(config: ServerConfig) -> Result<Server, Error> {
		let mut evtlp = reactor::Core::new().unwrap();

		let mut mining_config = config.mining_config.clone();
		let serv = Server::future(config, &evtlp.handle())?;
		if mining_config.as_mut().unwrap().enable_mining {
			serv.start_miner(mining_config.unwrap());
		}

		let forever = Timer::default()
			.interval(time::Duration::from_secs(60))
			.for_each(move |_| {
				debug!(LOGGER, "event loop running");
				Ok(())
			})
			.map_err(|_| ());

		evtlp.run(forever).unwrap();
		Ok(serv)
	}

	/// Instantiates a new server associated with the provided future reactor.
	pub fn future(mut config: ServerConfig, evt_handle: &reactor::Handle) -> Result<Server, Error> {
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
			_ => pow::mine_genesis_block(config.mining_config.clone())?,
		};
		info!(
			LOGGER,
			"Starting server, genesis block: {}",
			genesis.hash(),
		);

		let shared_chain = Arc::new(chain::Chain::init(
			config.db_root.clone(),
			chain_adapter.clone(),
			genesis.clone(),
			pow::verify_size,
		)?);

		pool_adapter.set_chain(shared_chain.clone());

		let net_adapter = Arc::new(NetToChainAdapter::new(
			shared_chain.clone(),
			tx_pool.clone(),
		));

		let p2p_server = Arc::new(p2p::Server::new(
			config.db_root.clone(),
			config.capabilities,
			config.p2p_config.unwrap(),
			net_adapter.clone(),
			genesis.hash(),
		)?);
		chain_adapter.init(p2p_server.clone());
		pool_net_adapter.init(p2p_server.clone());
		net_adapter.init(p2p_server.clone());

		let seed = seed::Seeder::new(config.capabilities, p2p_server.clone());
		match config.seeding_type.clone() {
			Seeding::None => {
				warn!(
					LOGGER,
					"No seed(s) configured, will stay solo until connected to"
				);
				seed.connect_and_monitor(
					evt_handle.clone(),
					seed::predefined_seeds(vec![]),
				);
			}
			Seeding::List => {
				seed.connect_and_monitor(
					evt_handle.clone(),
					seed::predefined_seeds(config.seeds.as_mut().unwrap().clone()),
				);
			}
			Seeding::WebStatic => {
				seed.connect_and_monitor(
					evt_handle.clone(),
					seed::web_seeds(evt_handle.clone()),
				);
			}
			_ => {}
		}

		sync::run_sync(
			net_adapter.clone(),
			p2p_server.clone(),
			shared_chain.clone(),
			);

		evt_handle.spawn(p2p_server.start(evt_handle.clone()).map_err(|_| ()));

		info!(LOGGER, "Starting rest apis at: {}", &config.api_http_addr);

		api::start_rest_apis(
			config.api_http_addr.clone(),
			shared_chain.clone(),
			tx_pool.clone(),
			p2p_server.clone(),
		);

		warn!(LOGGER, "Grin server started.");
		Ok(Server {
			config: config,
			evt_handle: evt_handle.clone(),
			p2p: p2p_server,
			chain: shared_chain,
			tx_pool: tx_pool,
			net_adapter: net_adapter,
		})
	}

	/// Asks the server to connect to a peer at the provided network address.
	pub fn connect_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let handle = self.evt_handle.clone();
		handle.spawn(
			self.p2p
				.connect_peer(addr, handle.clone())
				.map(|_| ())
				.map_err(|_| ()),
		);
		Ok(())
	}

	/// Number of peers
	pub fn peer_count(&self) -> u32 {
		self.p2p.peer_count()
	}

	/// Start mining for blocks on a separate thread. Uses toy miner by default,
	/// mostly for testing, but can also load a plugin from cuckoo-miner
	pub fn start_miner(&self, config: pow::types::MinerConfig) {
		let cuckoo_size = global::sizeshift();
		let proof_size = global::proofsize();
		let net_adapter = self.net_adapter.clone();

		let mut miner = miner::Miner::new(config.clone(), self.chain.clone(), self.tx_pool.clone());
		miner.set_debug_output_id(format!("Port {}", self.config.p2p_config.unwrap().port));
		thread::spawn(move || {
			// TODO push this down in the run loop so miner gets paused anytime we
			// decide to sync again
			let secs_5 = time::Duration::from_secs(5);
			while net_adapter.is_syncing() {
				thread::sleep(secs_5);
			}
			miner.run_loop(config.clone(), cuckoo_size as u32, proof_size);
		});
	}

	/// The chain head
	pub fn head(&self) -> chain::Tip {
		self.chain.head().unwrap()
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
}
