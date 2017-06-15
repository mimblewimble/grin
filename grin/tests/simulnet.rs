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

extern crate env_logger;
extern crate futures;
extern crate tokio_core;
extern crate tokio_timer;

use std::io;
use std::thread;
use std::time;
use std::default::Default;

use futures::{Future, Poll, Async};
use futures::task::park;
use tokio_core::reactor;
use tokio_timer::Timer;

/// Errors that can be returned by an ApiEndpoint implementation.
#[derive(Debug)]
pub enum Error {
	Internal(String),
	Argument(String),
	NotFound,
}

/// A top-level container to hold everything that might be running
/// on a server, i.e. server, wallet in send or recieve mode

struct LocalServerContainer {
    pub server : grin::Server,

    pub enable_mining: bool,
    pub enable_wallet: bool,
}

impl LocalServerContainer {
    pub fn new(api_addr:String, server_port: u16, event_loop: &reactor::Core) -> Result<LocalServerContainer, Error> {
      let mut s = grin::Server::future(
            grin::ServerConfig{
                api_http_addr: api_addr,
                db_root: format!("target/test_servers/server-{}/grin-prop", server_port),
                cuckoo_size: 12,
                p2p_config: p2p::P2PConfig{port: server_port, ..p2p::P2PConfig::default()},
                ..Default::default()
            }, &event_loop.handle()).unwrap();
       Ok((LocalServerContainer {
           server: s,
           enable_mining: false,
           enable_wallet: false,
       }))
    }
}

struct LocalServerContainerPool {
    event_loop: reactor::Core,

    base_http_addr: String, 
    base_port_server: u16,
    base_port_api: u16,
    server_containers: Vec<LocalServerContainer>,    
}

impl LocalServerContainerPool {
    pub fn new() -> Result<LocalServerContainerPool, Error> {
      let servers = Vec::new();
      let mut evtlp = reactor::Core::new().unwrap();

      Ok((LocalServerContainerPool{
        event_loop: evtlp, 
        base_http_addr : String::from("0.0.0.0"),
        base_port_server: 10000,
        base_port_api: 20000,
        server_containers: servers,
      }))
    }

    pub fn create_server(&mut self, enable_mining:bool) {

        let server_port = self.base_port_server+self.server_containers.len() as u16;
        let api_port = self.base_port_api+self.server_containers.len() as u16;

        let api_addr = format!("{}:{}", self.base_http_addr, api_port);

        let mut server_container = LocalServerContainer::new(api_addr, server_port, &self.event_loop).unwrap();
            
        server_container.enable_mining = enable_mining;
        
        self.server_containers.push(server_container);
    }

    /// Connects every server to each other as peers
    /// 

    pub fn connect_all_peers(&self){
        /// just pull out all currently active servers, build a list,
        /// and feed into all servers

        let mut server_addresses:Vec<String> = Vec::new();      
        for s in &self.server_containers {
            let server_address = format!("{}:{}", 
                                     s.server.config.p2p_config.host, 
                                     s.server.config.p2p_config.port);
            server_addresses.push(server_address);
        }

        for a in server_addresses {
           for s in &self.server_containers {
              if format!("{}", s.server.config.p2p_config.host) != a {
                  s.server.connect_peer(a.parse().unwrap()).unwrap();       
              } 
           }
        }
    }

    /// Starts all servers, with or without mining

    fn start_all_servers(&mut self) {
        for s in &self.server_containers {
            if s.enable_mining == true {
               let mut miner_config = grin::MinerConfig{
                  enable_mining: true,
                  burn_reward: true,
                  ..Default::default()
                };
                println!("Starting Miner on port {}", s.server.config.p2p_config.port);
                s.server.start_miner(miner_config);        
            }
        }
        self.event_loop.run(Timer::default().sleep(time::Duration::from_secs(30)).and_then(|_| {
          //for s in self.servers {  
            // occasionally 2 peers will connect to each other at the same time
            //assert!(s.peer_count() >= 4);
          //}
          Ok(())
        }));
    }
}

#[test]
fn simulate_much_mining(){
    println!("I'm here.");
    let num_servers=5;
    
    let mut server_pool = LocalServerContainerPool::new().unwrap();
    for n in 0..num_servers {
        server_pool.create_server(false);
    }

    server_pool.connect_all_peers();
    server_pool.start_all_servers();
  
    panic!("ouch");
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
