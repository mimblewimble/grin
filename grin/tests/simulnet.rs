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

extern crate grin_grin as grin;
extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate grin_chain as chain;
extern crate grin_api as api;
extern crate grin_wallet as wallet;
extern crate secp256k1zkp as secp;
extern crate tiny_keccak;

extern crate env_logger;
extern crate futures;
extern crate tokio_core;
extern crate tokio_timer;

use std::sync::{Arc, Mutex, RwLock};

mod framework;

use std::thread;
use std::time;
use std::default::Default;
use std::cell::RefCell;

use futures::{Future, Poll, Async};
use futures::task::park;
use tokio_core::reactor;
use tokio_timer::Timer;

use framework::{LocalServerContainer, LocalServerContainerConfig,
                LocalServerContainerPoolConfig, LocalServerContainerPool};

/// Testing the frameworks by starting a fresh server, creating a genesis
/// Block and mining into a wallet for a bit

#[test]
fn basic_genesis_mine(){
    env_logger::init();

    framework::clean_all_output();

    //Create a server pool
    let mut pool_config = LocalServerContainerPoolConfig::default();
    pool_config.base_name = format!("my_pool");
    pool_config.run_length_in_seconds = 30;

    let mut pool = LocalServerContainerPool::new(pool_config);

    //Create a server to add into the pool
    let mut server_config = LocalServerContainerConfig::default();
    server_config.start_miner=true;
    server_config.start_wallet=true;

    pool.create_server(server_config);

    pool.run_all_servers();

}

#[test]
fn framework_scratch (){

    framework::clean_all_output();

    //Create a server pool
    let mut pool_config = LocalServerContainerPoolConfig::default();
    pool_config.base_name = format!("my_pool");
    pool_config.run_length_in_seconds = 5;

    let mut pool = LocalServerContainerPool::new(pool_config);

    //Create a server to add into the pool
    let mut server_config = LocalServerContainerConfig::default();
    server_config.start_miner=true;
    server_config.start_wallet=true;

    for i in 0..5 {
        pool.create_server(server_config.clone());
    }

    pool.run_all_servers();


}

/// Create a network of 5 servers and mine a block, verifying that the block
/// gets propagated to all.
#[test]
fn simulate_block_propagation() {
  env_logger::init();

  let mut evtlp = reactor::Core::new().unwrap();
  let handle = evtlp.handle();

  let miner_config = grin::MinerConfig{
    enable_mining: true,
    burn_reward: true,
    ..Default::default()
  };

  // instantiates 5 servers on different ports
  let mut servers = vec![];
  for n in 0..5 {
      let s = grin::Server::future(
          grin::ServerConfig{
            api_http_addr: format!("127.0.0.1:{}", 20000+n),
            db_root: format!("target/grin-prop-{}", n),
            cuckoo_size: 12,
            p2p_config: p2p::P2PConfig{port: 10000+n, ..p2p::P2PConfig::default()},
            ..Default::default()
          }, &handle).unwrap();
      servers.push(s);
  }

  // everyone connects to everyone else
  for n in 0..5 {
    for m in 0..5 {
      if m == n { continue }
      let addr = format!("{}:{}", "127.0.0.1", 10000+m);
      servers[n].connect_peer(addr.parse().unwrap()).unwrap();
    }
  }

  // start mining
  servers[0].start_miner(miner_config);
  let original_height = servers[0].head().height;

  // monitor for a change of head on a different server and check whether
  // chain height has changed
  evtlp.run(change(&servers[4]).and_then(|tip| {
    assert!(tip.height == original_height+1);
    Ok(())
  }));
}

/// Creates 2 different disconnected servers, mine a few blocks on one, connect
/// them and check that the 2nd gets all the blocks
#[test]
fn simulate_full_sync() {
  env_logger::init();

  let mut evtlp = reactor::Core::new().unwrap();
  let handle = evtlp.handle();

  let miner_config = grin::MinerConfig{
    enable_mining: true,
    burn_reward: true,
    ..Default::default()
  };

  // instantiates 2 servers on different ports
  let mut servers = vec![];
  for n in 0..2 {
      let s = grin::Server::future(
          grin::ServerConfig{
            db_root: format!("target/grin-sync-{}", n),
            cuckoo_size: 12,
            p2p_config: p2p::P2PConfig{port: 11000+n, ..p2p::P2PConfig::default()},
            ..Default::default()
          }, &handle).unwrap();
      servers.push(s);
  }

  // mine a few blocks on server 1
  servers[0].start_miner(miner_config);
  thread::sleep(time::Duration::from_secs(15));

  // connect 1 and 2
  let addr = format!("{}:{}", "127.0.0.1", 11001);
  servers[0].connect_peer(addr.parse().unwrap()).unwrap();

  // 2 should get blocks
  evtlp.run(change(&servers[1]));
}

/// Creates 5 servers, one being a seed and check that through peer address
/// messages they all end up connected.
#[test]
fn simulate_seeding() {
  env_logger::init();

  let mut evtlp = reactor::Core::new().unwrap();
  let handle = evtlp.handle();

  // instantiates 5 servers on different ports, with 0 as a seed
  let mut servers = vec![];
  for n in 0..5 {
      let s = grin::Server::future(
          grin::ServerConfig{
            db_root: format!("target/grin-seed-{}", n),
            cuckoo_size: 12,
            p2p_config: p2p::P2PConfig{port: 12000+n, ..p2p::P2PConfig::default()},
            seeding_type: grin::Seeding::List(vec!["127.0.0.1:12000".to_string()]),
            ..Default::default()
          }, &handle).unwrap();
      servers.push(s);
  }

  // wait a bit and check all servers are now connected
  evtlp.run(Timer::default().sleep(time::Duration::from_secs(30)).and_then(|_| {
    for s in servers {
      // occasionally 2 peers will connect to each other at the same time
      assert!(s.peer_count() >= 4);
    }
    Ok(())
  }));
}

// Builds the change future, monitoring for a change of head on the provided server
fn change<'a>(s: &'a grin::Server) -> HeadChange<'a> {
  let start_head = s.head();
  HeadChange {
    server: s,
    original: start_head,
  }
}

/// Future that monitors when a server has had its head updated. Current
/// implementation isn't optimized, only use for tests.
struct HeadChange<'a> {
  server: &'a grin::Server,
  original:  chain::Tip,
}

impl<'a> Future for HeadChange<'a> {
  type Item = chain::Tip;
  type Error = ();

  fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
    let new_head = self.server.head();
    if new_head.last_block_h != self.original.last_block_h {
      Ok(Async::Ready(new_head))
    } else {
      // egregious polling, asking the task to schedule us every iteration
      park().unpark();
      Ok(Async::NotReady)
    }
  }
}
