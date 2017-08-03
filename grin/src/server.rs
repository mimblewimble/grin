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
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time;

use futures::{future, Future, Stream};
use tokio_core::reactor;
use tokio_timer::Timer;

use adapters::*;
use api;
use chain;
use chain::ChainStore;
use core::{self, consensus};
use core::core::hash::Hashed;
use core::pow::cuckoo;
use core::pow::MiningWorker;
use miner;
use p2p;
use pool;
use seed;
use store;
use sync;
use types::*;

use plugin::PluginMiner;

/// Grin server holding internal structures.
pub struct Server {
	pub config: ServerConfig,
	evt_handle: reactor::Handle,
	/// handle to our network server
	p2p: Arc<p2p::Server>,
	/// data store access
	chain: Arc<chain::Chain>,
	/// in-memory transaction pool
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
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
				debug!("event loop running");
				Ok(())
			})
			.map_err(|_| ());

		evtlp.run(forever).unwrap();
		Ok(serv)
	}

	/// Instantiates a new server associated with the provided future reactor.
	pub fn future(mut config: ServerConfig, evt_handle: &reactor::Handle) -> Result<Server, Error> {

		let pool_adapter = Arc::new(PoolToChainAdapter::new());
		let tx_pool = Arc::new(RwLock::new(pool::TransactionPool::new(pool_adapter.clone())));

		let chain_adapter = Arc::new(ChainToPoolAndNetAdapter::new(tx_pool.clone()));
		let shared_chain = Arc::new(chain::Chain::init(config.test_mode,
		                                               config.db_root.clone(),
		                                               chain_adapter.clone())?);
		pool_adapter.set_chain(shared_chain.clone());

		let peer_store = Arc::new(p2p::PeerStore::new(config.db_root.clone())?);
		let net_adapter = Arc::new(NetToChainAdapter::new(config.test_mode,
		                                                  shared_chain.clone(),
		                                                  tx_pool.clone(),
		                                                  peer_store.clone()));
		let p2p_server =
			Arc::new(p2p::Server::new(config.capabilities, config.p2p_config.unwrap(), net_adapter.clone()));
		chain_adapter.init(p2p_server.clone());

		let seed = seed::Seeder::new(config.capabilities, peer_store.clone(), p2p_server.clone());
		match config.seeding_type.clone() {
			Seeding::None => {}
			Seeding::List => {
				seed.connect_and_monitor(evt_handle.clone(), seed::predefined_seeds(config.seeds.as_mut().unwrap().clone()));
			}
			Seeding::WebStatic => {
				seed.connect_and_monitor(evt_handle.clone(), seed::web_seeds(evt_handle.clone()));
			}
		}

		let sync = sync::Syncer::new(shared_chain.clone(), p2p_server.clone());
		net_adapter.start_sync(sync);

		evt_handle.spawn(p2p_server.start(evt_handle.clone()).map_err(|_| ()));

		info!("Starting rest apis at: {}", &config.api_http_addr);

		api::start_rest_apis(config.api_http_addr.clone(),
		                     shared_chain.clone(),
		                     tx_pool.clone());

		warn!("Grin server started.");
		Ok(Server {
			config: config,
			evt_handle: evt_handle.clone(),
			p2p: p2p_server,
			chain: shared_chain,
			tx_pool: tx_pool,
		})
	}

	/// Asks the server to connect to a peer at the provided network address.
	pub fn connect_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let handle = self.evt_handle.clone();
		handle.spawn(self.p2p.connect_peer(addr, handle.clone()).map(|_| ()).map_err(|_| ()));
		Ok(())
	}

	pub fn peer_count(&self) -> u32 {
		self.p2p.peer_count()
	}

	/// Start mining for blocks on a separate thread. Uses toy miner by default,
	/// mostly for testing, but can also load a plugin from cuckoo-miner
	pub fn start_miner(&self, config: MinerConfig) {
		let cuckoo_size = match self.config.test_mode {
			true => consensus::TEST_SIZESHIFT as u32,
			false => consensus::DEFAULT_SIZESHIFT as u32,
		}; 
		let mut miner = miner::Miner::new(config.clone(), self.chain.clone(), self.tx_pool.clone());
		miner.set_debug_output_id(format!("Port {}",self.config.p2p_config.unwrap().port));
		let server_config = self.config.clone();
		thread::spawn(move || {
			miner.run_loop(config.clone(), server_config, cuckoo_size);		
		});
	}

	pub fn head(&self) -> chain::Tip {
		self.chain.head().unwrap()
	}

	/// Returns a set of stats about this server. This and the ServerStats structure
	/// can be updated over time to include any information needed by tests or other
	/// consumers

	pub fn get_server_stats(&self) -> Result<ServerStats, Error>{
		Ok(ServerStats{
			peer_count: self.peer_count(),
			head: self.head(),
		})
	}
}
