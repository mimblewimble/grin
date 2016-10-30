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

//! Grin server implementation, accepts incoming connections and connects to
//! other peers in the network.

use std::cell::RefCell;
use std::io;
use std::net::SocketAddr;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use mioco;
use mioco::tcp::{TcpListener, TcpStream};

use core::ser::Error;
use handshake::Handshake;
use peer::Peer;
use types::*;

pub const DEFAULT_LISTEN_ADDR: &'static str = "127.0.0.1:3414";

// replace with some config lookup or something
fn listen_addr() -> SocketAddr {
	FromStr::from_str(DEFAULT_LISTEN_ADDR).unwrap()
}

struct DummyAdapter {}
impl NetAdapter for DummyAdapter {}

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
  peers: RwLock<Vec<Arc<Peer>>>,
}

// TODO TLS
impl Server {
  pub fn new() -> Server {
    Server{peers: RwLock::new(Vec::new())}
  }
	/// Creates a new p2p server. Opens a TCP port to allow incoming
	/// connections and starts the bootstrapping process to find peers.
	pub fn start(& self) -> Result<(), Error> {
		mioco::spawn(move || -> io::Result<()> {
			let addr = DEFAULT_LISTEN_ADDR.parse().unwrap();
			let listener = try!(TcpListener::bind(&addr));
			warn!("P2P server started on {}", addr);

			let hs = Arc::new(Handshake::new());

			loop {
				let conn = try!(listener.accept());
				let hs = hs.clone();

				mioco::spawn(move || -> io::Result<()> {
          let peer = try!(Peer::connect(conn, &hs).map_err(|_| io::Error::last_os_error()));
          let wpeer = Arc::new(peer);
          // {
          //   let mut peers = self.peers.write().unwrap();
          //   peers.push(wpeer.clone());
          // }
          if let Some(err) = wpeer.run(&DummyAdapter{}) {
						error!("{:?}", err);
					}
					Ok(())
				});
			}
			Ok(())
		});
		Ok(())
	}

  pub fn stop(&self) {
    let peers = self.peers.write().unwrap();
    for p in peers.deref() {
      p.stop();
    }
  }

	/// Simulates an unrelated client connecting to our server. Mostly used for
	/// tests.
	pub fn connect_as_client(addr: SocketAddr) -> Result<Peer, Error> {
		let tcp_client = TcpStream::connect(&addr).unwrap();
		Peer::accept(tcp_client, &Handshake::new())
	}
}
