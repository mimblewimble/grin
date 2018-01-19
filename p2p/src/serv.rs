// Copyright 2016-2018 The Grin Developers
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

use std::net::{TcpListener, TcpStream};

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	config: P2PConfig,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	peers: Peers,
}

unsafe impl Sync for Server {}
unsafe impl Send for Server {}

// TODO TLS
impl Server {
	/// Creates a new idle p2p server with no peers
	pub fn new(
		db_root: String,
		capab: Capabilities,
		config: P2PConfig,
		adapter: Arc<ChainAdapter>,
		genesis: Hash,
	) -> Result<Server, Error> {
		Ok(Server {
			config: config,
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis)),
			peers: Peers::new(PeerStore::new(db_root)?, adapter),
		})
	}

	pub fn listen(&self) -> Result<(), Error> {
		let addr = SocketAddr::new(self.config.host, self.config.port);
		let listener = TcpListener::bind(addr)?;

		for stream in listener.incoming() {
			match stream {
				Ok(stream) => {
					if !self.check_banned(stream) {
						self.handle_new_peer(stream);
					}
				}
				Err(e) => {
					warn!(LOGGER, "Couldn't establish new client connection: {:?}", e);
				}
			}
		}
	}

	fn handle_new_peer(&self, stream: TcpStream) {
		let total_diff = self.peers.total_difficulty();

		// accept the peer and add it to the server map
		let peer = Peer::accept_new(
			stream,
			capab,
			total_diff,
			&handshake.clone(),
			Arc::new(peers.clone()),
			);
		let added = peers.add_connected(peers2, accept);
		let peer = added.read().unwrap();
		thread::new(move || {
			peer.run();
		});
	}

	fn check_banned(&self, stream: TcpStream) -> bool {
		// peer has been banned, go away!
		if let Ok(peer_addr) = stream.peer_addr() {
			if self.peers.is_banned(peer_addr) {
				debug!(LOGGER, "Peer {} banned, refusing connection.", peer_addr);
				if let Err(e) = conn.shutdown(Shutdown::Both) {
					debug!(LOGGER, "Error shutting down conn: {:?}", e);
				}
				return true;
			}
		}
		false
	}
}
