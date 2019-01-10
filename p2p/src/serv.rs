// Copyright 2018 The Grin Developers
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

use std::collections::HashMap;
use std::fs::File;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;
use std::{io, thread};

use crate::lmdb;

use crate::core::core;
use crate::core::core::hash::Hash;
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::handshake::Handshake;
use crate::peer::Peer;
use crate::peers::Peers;
use crate::store::PeerStore;
use crate::types::{Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, TxHashSetRead, ReasonForBan};
use crate::util::{Mutex, StopState};
use chrono::prelude::{DateTime, Utc};

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	pub config: P2PConfig,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	pub peers: Arc<Peers>,
	stop_state: Arc<Mutex<StopState>>,
}

// TODO TLS
impl Server {
	/// Creates a new idle p2p server with no peers
	pub fn new(
		db_env: Arc<lmdb::Environment>,
		capab: Capabilities,
		config: P2PConfig,
		adapter: Arc<dyn ChainAdapter>,
		genesis: Hash,
		stop_state: Arc<Mutex<StopState>>,
	) -> Result<Server, Error> {
		Ok(Server {
			config: config.clone(),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Arc::new(Peers::new(PeerStore::new(db_env)?, adapter, config)),
			stop_state,
		})
	}

	/// Starts a new TCP server and listen to incoming connections. This is a
	/// blocking call until the TCP server stops.
	pub fn listen(&self) -> Result<(), Error> {
		// start TCP listener and handle incoming connections
		let addr = SocketAddr::new(self.config.host, self.config.port);
		let listener = TcpListener::bind(addr)?;
		listener.set_nonblocking(true)?;

		let mut connected_sockets: HashMap<SocketAddr, TcpStream> = HashMap::new();

		let sleep_time = Duration::from_millis(1);
		loop {
			// Pause peer ingress connection request. Only for tests.
			if self.stop_state.lock().is_paused() {
				thread::sleep(Duration::from_secs(1));
				continue;
			}

			match listener.accept() {
				Ok((stream, peer_addr)) => {
					if !self.check_banned(&stream) {
						let sc = stream.try_clone();
						if let Err(e) = self.handle_new_peer(stream) {
							warn!("Error accepting peer {}: {:?}", peer_addr.to_string(), e);
							let _ = self.peers.add_banned(peer_addr, ReasonForBan::BadHandshake);
						} else if let Ok(s) = sc {
							connected_sockets.insert(peer_addr, s);
						}
					}
					// if any active socket not in our peers list, close it
					self.clean_lost_sockets(&mut connected_sockets);
				}
				Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
					// nothing to do, will retry in next iteration
				}
				Err(e) => {
					warn!("Couldn't establish new client connection: {:?}", e);
				}
			}
			if self.stop_state.lock().is_stopped() {
				break;
			}
			thread::sleep(sleep_time);
		}
		Ok(())
	}

	/// Asks the server to connect to a new peer. Directly returns the peer if
	/// we're already connected to the provided address.
	pub fn connect(&self, addr: &SocketAddr) -> Result<Arc<Peer>, Error> {
		if Peer::is_denied(&self.config, &addr) {
			debug!("connect_peer: peer {} denied, not connecting.", addr);
			return Err(Error::ConnectionClose);
		}

		if global::is_production_mode() {
			let hs = self.handshake.clone();
			let addrs = hs.addrs.read();
			if addrs.contains(&addr) {
				debug!("connect: ignore connecting to PeerWithSelf, addr: {}", addr);
				return Err(Error::PeerWithSelf);
			}
		}

		if let Some(p) = self.peers.get_connected_peer(addr) {
			// if we're already connected to the addr, just return the peer
			trace!("connect_peer: already connected {}", addr);
			return Ok(p);
		}

		trace!(
			"connect_peer: on {}:{}. connecting to {}",
			self.config.host,
			self.config.port,
			addr
		);
		match TcpStream::connect_timeout(addr, Duration::from_secs(10)) {
			Ok(mut stream) => {
				let addr = SocketAddr::new(self.config.host, self.config.port);
				let total_diff = self.peers.total_difficulty();

				let mut peer = Peer::connect(
					&mut stream,
					self.capabilities,
					total_diff,
					addr,
					&self.handshake,
					self.peers.clone(),
				)?;
				peer.start(stream);
				let peer = Arc::new(peer);
				self.peers.add_connected(peer.clone())?;
				Ok(peer)
			}
			Err(e) => {
				trace!(
					"connect_peer: on {}:{}. Could not connect to {}: {:?}",
					self.config.host,
					self.config.port,
					addr,
					e
				);
				Err(Error::Connection(e))
			}
		}
	}

	fn handle_new_peer(&self, mut stream: TcpStream) -> Result<(), Error> {
		let total_diff = self.peers.total_difficulty();

		// accept the peer and add it to the server map
		let mut peer = Peer::accept(
			&mut stream,
			self.capabilities,
			total_diff,
			&self.handshake,
			self.peers.clone(),
		)?;
		peer.start(stream);
		self.peers.add_connected(Arc::new(peer))?;
		Ok(())
	}

	fn check_banned(&self, stream: &TcpStream) -> bool {
		// peer has been banned, go away!
		if let Ok(peer_addr) = stream.peer_addr() {
			if self.peers.is_banned(peer_addr) {
				debug!("Peer {} banned, refusing connection.", peer_addr);
				if let Err(e) = stream.shutdown(Shutdown::Both) {
					debug!("Error shutting down conn: {:?}", e);
				}
				return true;
			}
		}
		false
	}

	/// For all kinds of exception cases, the node could accepted / initiated a peer connection successfully but
	/// failed on the Handshake protocol communication, or a connected peer was closed but without a successful
	/// clean-up on its socket, that will cause this connected (on TcpStream) peer becomes so-called "invisible" peer!
	/// i.e. a peer not included in the 'self.peers.peers' hashmap. This "invisible" peer will cause some security
	/// concern because it still can send something to this node, but without enough visibility as other connected peers.
	/// Another impact is these connections could never be closed, which make the node fully occupied by all such
	/// kind of connections and become un-connectable.
	/// This function can help to clean the peer connections which is "invisible" for this node.
	fn clean_lost_sockets(&self, sockets: &mut HashMap<SocketAddr, TcpStream>) {
		let mut lost_sockets: Vec<SocketAddr> = vec![];
		for (socket, stream) in sockets.iter() {
			if !self.peers.is_known_ip(&socket) {
				if let Ok(_) = stream.shutdown(Shutdown::Both) {
					debug!(
						"clean_lost_sockets: {} cleaned which's not in peers list",
						socket
					);
				}
				lost_sockets.push(socket.clone());
			}
		}

		for socket in lost_sockets {
			sockets.remove(&socket);
		}
	}

	pub fn stop(&self) {
		self.stop_state.lock().stop();
		self.peers.stop();
	}

	/// Pause means: stop all the current peers connection, only for tests.
	/// Note:
	/// 1. must pause the 'seed' thread also, to avoid the new egress peer connection
	/// 2. must pause the 'p2p-server' thread also, to avoid the new ingress peer connection.
	pub fn pause(&self) {
		self.peers.stop();
	}
}

/// A no-op network adapter used for testing.
pub struct DummyAdapter {}

impl ChainAdapter for DummyAdapter {
	fn total_difficulty(&self) -> Difficulty {
		Difficulty::min()
	}
	fn total_height(&self) -> u64 {
		0
	}
	fn get_transaction(&self, _h: Hash) -> Option<core::Transaction> {
		None
	}
	fn tx_kernel_received(&self, _h: Hash, _addr: SocketAddr) {}
	fn transaction_received(&self, _: core::Transaction, _stem: bool) {}
	fn compact_block_received(&self, _cb: core::CompactBlock, _addr: SocketAddr) -> bool {
		true
	}
	fn header_received(&self, _bh: core::BlockHeader, _addr: SocketAddr) -> bool {
		true
	}
	fn block_received(&self, _: core::Block, _: SocketAddr) -> bool {
		true
	}
	fn headers_received(&self, _: &[core::BlockHeader], _: SocketAddr) -> bool {
		true
	}
	fn locate_headers(&self, _: &[Hash]) -> Vec<core::BlockHeader> {
		vec![]
	}
	fn get_block(&self, _: Hash) -> Option<core::Block> {
		None
	}
	fn txhashset_read(&self, _h: Hash) -> Option<TxHashSetRead> {
		unimplemented!()
	}

	fn txhashset_receive_ready(&self) -> bool {
		false
	}

	fn txhashset_write(&self, _h: Hash, _txhashset_data: File, _peer_addr: SocketAddr) -> bool {
		false
	}

	fn txhashset_download_update(
		&self,
		_start_time: DateTime<Utc>,
		_downloaded_size: u64,
		_total_size: u64,
	) -> bool {
		false
	}
}

impl NetAdapter for DummyAdapter {
	fn find_peer_addrs(&self, _: Capabilities) -> Vec<SocketAddr> {
		vec![]
	}
	fn peer_addrs_received(&self, _: Vec<SocketAddr>) {}
	fn peer_difficulty(&self, _: SocketAddr, _: Difficulty, _: u64) {}
	fn is_banned(&self, _: SocketAddr) -> bool {
		false
	}
}
