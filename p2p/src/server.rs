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
//! other peers in the network, handling handshake and message receive/send.

use str::net::SocketAddr;
use std::str::FromStr;
use time::Duration;

use mioco::tcp::{TcpListener, TcpStream, Shutdown};

use core::ser::{serialize, deserialize};
use msg::*;

const DEFAULT_LISTEN_ADDR: &'static str = "127.0.0.1:555";
const PROTOCOL_VERSION: u32 = 1;
const USER_AGENT: &'static str = "MW/Grin 0.1";

// replace with some config lookup or something
fn listen_addr() -> SocketAddr {
	FromStr::from_str(DEFAULT_LISTEN_ADDR).unwrap()
}

/// The local representation of a remotely connected peer. Handles most
/// low-level network communication.
struct Peer {
	conn: TcpStream,
	reader: BufReader,
	capabilities: Capabilities,
	user_agent: String,
}

impl Peer {
	/// Create a new local peer instance connected to a remote peer with the
	/// provided TcpStream.
	fn new(conn: TcpStream) -> Peer {
		// don't wait on read for more than 2 seconds by default
		conn.set_read_timeout(Some(Duration::seconds(2)));

		Peer {
			conn: conn,
			reader: BufReader::new(conn),
		}
	}

	/// Handles connecting to a new remote peer, starting the version handshake.
	fn connect(&mut self) {
		serialize(self.conn,
		          &Hand {
			          version: PROTOCOL_VERSION,
			          capabilities: FULL_SYNC,
			          sender_addr: listen_addr(),
			          receiver_addr: conn.peer_addr(),
			          user_agent: USER_AGENT,
		          });
		let shake = deserialize(self.reader);
		if shake.version != 1 {
			self.close(ErrCodes::UNSUPPORTED_VERSION,
			           format!("Unsupported version: {}, ours: {})",
			                   shake.version,
			                   PROTOCOL_VERSION));
			return;
		}
		self.capabilities = shake.capabilities;
		self.user_agent = shake.user_agent;

		self.accept_loop();
	}

	/// Handles receiving a connection from a new remote peer that started the
	/// version handshake.
	fn handshake(&mut self) {}

	fn accept_loop(&mut self) {
		loop {
			let msg = deserialize(self.reader);
		}
	}

	fn close(err_code: u32, explanation: &'static str) {
		serialize(self.conn,
		          &Err {
			          code: err_code,
			          message: explanation,
		          });
		self.conn.shutdown(Shutdown::Both);
	}
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
						Peer::new(conn).connect();
					});
				}
			})
			.unwrap()
			.unwrap();
	}
}
