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
extern crate mioco;
extern crate env_logger;

use std::io;
use std::time;

#[test]
fn simulate_servers() {
  env_logger::init().unwrap();

  mioco::start(move || -> io::Result<()> {
    // instantiates 5 servers on different ports
    let mut servers = vec![];
    for n in 0..5 {
      let s = grin::Server::start(
          grin::ServerConfig{
            db_root: format!("target/grin-{}", n),
            cuckoo_size: 18,
            p2p_config: p2p::P2PConfig{port: 10000+n, ..p2p::P2PConfig::default()}
          }).unwrap();
      servers.push(s);
    }

    mioco::sleep(time::Duration::from_millis(100));

    // everyone connects to everyone else
    for n in 0..5 {
      for m in 0..5 {
        if m == n { continue }
        let addr = format!("{}:{}", "127.0.0.1", 10000+m);
        servers[n].connect_peer(addr.as_str()).unwrap();
        mioco::sleep(time::Duration::from_millis(100));
        println!("c {}", m);
      }
    }
    Ok(())
  });

}
