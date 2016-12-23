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

use std::io;
use std::thread;
use std::time;

use futures::{Future, Poll, Async};
use futures::task::park;
use tokio_core::reactor;

#[test]
fn simulate_servers() {
  env_logger::init().unwrap();

  let mut evtlp = reactor::Core::new().unwrap();
  let handle = evtlp.handle();

  // instantiates 5 servers on different ports
  let mut servers = vec![];
  for n in 0..5 {
      let s = grin::Server::future(
          grin::ServerConfig{
            db_root: format!("target/grin-{}", n),
            cuckoo_size: 12,
            p2p_config: p2p::P2PConfig{port: 10000+n, ..p2p::P2PConfig::default()}
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
  servers[0].start_miner();
  let original_height = servers[0].head().height;

  // monitor for a change of head on a different server and check we 
  evtlp.run(change(&servers[4]).and_then(|tip| {
    assert!(tip.height == original_height+1);
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

/// Future that monitors when a server has had its head updated.
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
