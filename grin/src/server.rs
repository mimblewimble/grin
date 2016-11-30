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

//! Grin server implementation, glues the different parts of the system (mostly the peer-to-peer server, the blockchain and the transaction pool) and acts as a facade.

use mioco;
use std::io;
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};
use std::thread;

use chain;
use chain::ChainStore;
use core;
use miner;
use p2p;

/// Errors than can be reported by a server implementation, mostly wraps underlying components errors.
#[derive(Debug)]
pub enum Error {
  /// Error when trying to add a block to the chain
  ChainErr(chain::pipe::Error),
  /// Peer connection error
  PeerErr(core::ser::Error),
  /// Data store error
	StoreErr(chain::types::Error),
}

/// Full server configuration, aggregating configurations required for the different components.
#[derive(Debug, Clone)]
pub struct ServerConfig {
  /// Directory under which the rocksdb stores will be created
  pub db_root: String,
  /// Allows overriding the default cuckoo cycle size
  pub cuckoo_size: u8,
  /// Configuration for the peer-to-peer server
  pub p2p_config: p2p::P2PConfig,
}

impl Default for ServerConfig {
  fn default() -> ServerConfig {
    ServerConfig{
      db_root: ".grin".to_string(),
      cuckoo_size: 0,
      p2p_config: p2p::P2PConfig::default(),
    }
  }
}

/// Grin server holding internal structures.
pub struct Server {
  config: ServerConfig,
  /// handle to our network server
  p2p: Arc<p2p::Server>,
  /// the reference copy of the current chain state
  chain_head: Arc<Mutex<chain::Tip>>,
  /// data store access
  chain_store: Arc<Mutex<chain::ChainStore>>,
}

impl Server {
  /// Instantiates and starts a new server.
  pub fn start(config: ServerConfig) -> Result<Server, Error> {
    let chain_store = try!(chain::store::ChainKVStore::new(config.db_root.clone()).map_err(&Error::StoreErr));

    // check if we have a head in store, otherwise the genesis block is it
    let head = match chain_store.head() {
      Ok(tip) => tip,
      Err(chain::types::Error::NotFoundErr) => {
        let mut gen = core::genesis::genesis();
        if config.cuckoo_size > 0 {
          gen.header.cuckoo_len = config.cuckoo_size;
        }
        let tip = chain::types::Tip::new(gen.hash());
        try!(chain_store.save_tip(&tip).map_err(&Error::StoreErr));
        tip
      },
      Err(e) => return Err(Error::StoreErr(e)),
    };

    let server = Arc::new(p2p::Server::new(config.p2p_config));
    let in_server = server.clone();
		mioco::spawn(move || -> io::Result<()> {
      try!(in_server.start().map_err(|_| io::Error::last_os_error()));
			Ok(())
		});

		warn!("Grin server started.");
    Ok(Server{
      config: config,
      p2p: server,
      chain_head: Arc::new(Mutex::new(head)),
      chain_store: Arc::new(Mutex::new(chain_store)),
    })
  }

  /// Asks the server to connect to a peer at the provided network address.
  pub fn connect_peer<A: ToSocketAddrs>(&self, addr: A) -> Result<(), Error> {
    self.p2p.connect_peer(addr).map_err(&Error::PeerErr)
  }

  /// Start mining for blocks on a separate thread. Relies on a toy miner, mostly for testing.
  pub fn start_miner(&self) {
    let miner = miner::Miner::new(self.chain_head.clone(), self.chain_store.clone());
    thread::spawn(move || {
      miner.run_loop();
    });
  }
}
