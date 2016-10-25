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

use mioco::tcp::{TcpListener, TcpStream, Shutdown};

use core::ser::{serialize, deserialize};
use msg::*;

const DEFAULT_LISTEN_ADDR: &'static str = "127.0.0.1:555";

// replace with some config lookup or something
fn listen_addr() -> SocketAddr {
	FromStr::from_str(DEFAULT_LISTEN_ADDR).unwrap()
}

pub struct Server {
}

impl Server {
	/// Creates a new p2p server. Opens a TCP port to allow incoming
	/// connections and starts the bootstrapping process to find peers.
	pub fn new() -> Server {
		mioco::start(|| -> io::Result<()> {
				let addr = "127.0.0.1:3414".parse().unwrap();
				let listener = try!(TcpListener::bind(&addr));
				info!("P2P server started on {}", addr);

				loop {
					let mut conn = try!(listener.accept());
					mioco::spawn(move || -> io::Result<()> {
						Peer::new(conn).handshake();
					});
				}
			})
			.unwrap()
			.unwrap();
	}
}
