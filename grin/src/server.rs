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
use miner;
use p2p;
use pool;
use seed;
use store;
use sync;
use types::*;

/// Grin server holding internal structures.
pub struct Server {
	pub config: ServerConfig,
	evt_handle: reactor::Handle,
	/// handle to our network server
	p2p: Arc<p2p::Server>,
	/// the reference copy of the current chain state
	chain_head: Arc<Mutex<chain::Tip>>,
	/// data store access
	chain_store: Arc<chain::ChainStore>,
	/// chain adapter to net, required for miner and anything that submits
	/// blocks
	chain_adapter: Arc<ChainToPoolAndNetAdapter>,
	/// in-memory transaction pool
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
}

impl Server {
	/// Instantiates and starts a new server.
	pub fn start(mut config: ServerConfig) -> Result<Server, Error> {
		check_config(&mut config);
		let mut evtlp = reactor::Core::new().unwrap();

		let mining_config = config.mining_config.clone();
		let serv = Server::future(config, &evtlp.handle())?;
		if mining_config.enable_mining {
			serv.start_miner(mining_config);
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
		check_config(&mut config);

		let (chain_store, head) = try!(store_head(&config));
		let shared_head = Arc::new(Mutex::new(head));

		let peer_store = Arc::new(p2p::PeerStore::new(config.db_root.clone())?);

		let pool_adapter = Arc::new(PoolToChainAdapter::new(shared_head.clone(),
		                                                    chain_store.clone()));
		let tx_pool = Arc::new(RwLock::new(pool::TransactionPool::new(pool_adapter)));

		let chain_adapter = Arc::new(ChainToPoolAndNetAdapter::new(tx_pool.clone()));
		let net_adapter = Arc::new(NetToChainAdapter::new(config.test_mode,
		                                                  shared_head.clone(),
		                                                  chain_store.clone(),
		                                                  chain_adapter.clone(),
		                                                  tx_pool.clone(),
		                                                  peer_store.clone()));
		let server =
			Arc::new(p2p::Server::new(config.capabilities, config.p2p_config, net_adapter.clone()));
		chain_adapter.init(server.clone());

		let seed = seed::Seeder::new(config.capabilities, peer_store.clone(), server.clone());
		match config.seeding_type.clone() {
			Seeding::None => {}
			Seeding::List(seeds) => {
				seed.connect_and_monitor(evt_handle.clone(), seed::predefined_seeds(seeds));
			}
			Seeding::WebStatic => {
				seed.connect_and_monitor(evt_handle.clone(), seed::web_seeds(evt_handle.clone()));
			}
		}

		let sync = sync::Syncer::new(chain_store.clone(), server.clone());
		net_adapter.start_sync(sync);

		evt_handle.spawn(server.start(evt_handle.clone()).map_err(|_| ()));

		info!("Starting rest apis at: {}", &config.api_http_addr);

		api::start_rest_apis(config.api_http_addr.clone(),
		                     chain_store.clone(),
		                     shared_head.clone(),
		                     tx_pool.clone());

		warn!("Grin server started.");
		Ok(Server {
			config: config,
			evt_handle: evt_handle.clone(),
			p2p: server,
			chain_head: shared_head,
			chain_store: chain_store,
			chain_adapter: chain_adapter,
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

	/// Start mining for blocks on a separate thread. Relies on a toy miner,
	/// mostly for testing.
	pub fn start_miner(&self, config: MinerConfig) {
		let mut miner = miner::Miner::new(config,
		                              self.chain_head.clone(),
		                              self.chain_store.clone(),
		                              self.chain_adapter.clone(),
		                              self.tx_pool.clone());
		miner.set_debug_output_id(format!("Port {}",self.config.p2p_config.port));
		thread::spawn(move || {
			miner.run_loop();
		});
	}

	pub fn head(&self) -> chain::Tip {
		let head = self.chain_head.clone();
		let h = head.lock().unwrap();
		h.clone()
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

// Helper function to create the chain storage and check if it already has a
// genesis block
fn store_head(config: &ServerConfig)
              -> Result<(Arc<chain::store::ChainKVStore>, chain::Tip), Error> {
	let chain_store = try!(chain::store::ChainKVStore::new(config.db_root.clone())
		.map_err(&Error::Store));

	// check if we have a head in store, otherwise the genesis block is it
	let head = match chain_store.head() {
		Ok(tip) => tip,
		Err(store::Error::NotFoundErr) => {
			info!("No genesis block found, creating and saving one.");
			let mut gen = core::genesis::genesis();
			let diff = gen.header.difficulty.clone();
			core::pow::pow_size(&mut gen.header,
			                    diff,
			                    config.mining_config.cuckoo_size as u32)
				.unwrap();
			chain_store.save_block(&gen).map_err(&Error::Store)?;
			let tip = chain::types::Tip::new(gen.hash());
			chain_store.save_head(&tip).map_err(&Error::Store)?;
			info!("Saved genesis block with hash {}", gen.hash());
			tip
		}
		Err(e) => return Err(Error::Store(e)),
	};

	let head = chain_store.head()?;
	let head_header = chain_store.head_header()?;
	info!("Starting server with head {} at {} and header head {} at {}",
	      head.last_block_h,
	      head.height,
	      head_header.hash(),
	      head_header.height);

	Ok((Arc::new(chain_store), head))
}

fn check_config(config: &mut ServerConfig) {
	// applying test/normal config
	config.mining_config.cuckoo_size = if config.test_mode {
		consensus::TEST_SIZESHIFT as u32
	} else {
		consensus::DEFAULT_SIZESHIFT as u32
	};
}
