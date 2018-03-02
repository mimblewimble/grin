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

use std::fs::File;
use std::io;
use std::net::{TcpListener, TcpStream, SocketAddr, Shutdown};
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use handshake::Handshake;
use peer::Peer;
use peers::Peers;
use store::PeerStore;
use types::*;
use util::LOGGER;

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	config: P2PConfig,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	pub peers: Arc<Peers>,
	stop: Arc<AtomicBool>,
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
		stop: Arc<AtomicBool>,
	) -> Result<Server, Error> {

		Ok(Server {
			config: config.clone(),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Arc::new(Peers::new(PeerStore::new(db_root)?, adapter, config)),
			stop: stop,
		})
	}

	/// Starts a new TCP server and listen to incoming connections. This is a
	/// blocking call until the TCP server stops.
	pub fn listen(&self) -> Result<(), Error> {
		// start peer monitoring thread
		let peers_inner = self.peers.clone();
		let stop = self.stop.clone();
		let _ = thread::Builder::new().name("p2p-monitor".to_string()).spawn(move || {
			loop {
				let total_diff = peers_inner.total_difficulty();
				let total_height = peers_inner.total_height();
				peers_inner.check_all(total_diff, total_height);
				thread::sleep(Duration::from_secs(10));
				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});

		// start TCP listener and handle incoming connections
		let addr = SocketAddr::new(self.config.host, self.config.port);
		let listener = TcpListener::bind(addr)?;
		listener.set_nonblocking(true)?;

		let sleep_time = Duration::from_millis(1);
		loop {
			match listener.accept() {
				Ok((stream, peer_addr)) => {
					if !self.check_banned(&stream) {
						if let Err(e) = self.handle_new_peer(stream) {
							debug!(
								LOGGER,
								"Error accepting peer {}: {:?}",
								peer_addr.to_string(),
								e);
						}
					}
				}
				Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
					// nothing to do, will retry in next iteration
				}
				Err(e) => {
					warn!(LOGGER, "Couldn't establish new client connection: {:?}", e);
				}
			}
			if self.stop.load(Ordering::Relaxed) {
				break;
			}
			thread::sleep(sleep_time);
		}
		Ok(())
	}

	/// Asks the server to connect to a new peer. Directly returns the peer if
	/// we're already connected to the provided address.
	pub fn connect(&self, addr: &SocketAddr) -> Result<Arc<RwLock<Peer>>, Error> {
		if Peer::is_denied(&self.config, &addr) {
			debug!(LOGGER, "Peer {} denied, not connecting.", addr);
			return Err(Error::ConnectionClose);
		}

		if let Some(p) = self.peers.get_connected_peer(addr) {
			// if we're already connected to the addr, just return the peer
			debug!(LOGGER, "connect_peer: already connected {}", addr);
			return Ok(p);
		}

		debug!(LOGGER, "connect_peer: connecting to {}", addr);
		match TcpStream::connect_timeout(addr, Duration::from_secs(10)) {
			Ok(mut stream) => {
				let addr = SocketAddr::new(self.config.host, self.config.port);
				let total_diff = self.peers.total_difficulty();

				let peer = Peer::connect(
					&mut stream,
					self.capabilities,
					total_diff,
					addr,
					&self.handshake,
					self.peers.clone(),
				)?;
				let added = self.peers.add_connected(peer);
				{
					let mut peer = added.write().unwrap();
					peer.start(stream);
				}
				Ok(added)
			}
			Err(e) => {
				debug!(LOGGER, "Could not connect to {}: {:?}", addr, e);
				Err(Error::Connection(e))
			}
		}
	}

	fn handle_new_peer(&self, mut stream: TcpStream) -> Result<(), Error> {
		let total_diff = self.peers.total_difficulty();

		// accept the peer and add it to the server map
		let peer = Peer::accept(
			&mut stream,
			self.capabilities,
			total_diff,
			&self.handshake,
			self.peers.clone(),
		)?;
		let added = self.peers.add_connected(peer);
		let mut peer = added.write().unwrap();
		peer.start(stream);
		Ok(())
	}

	fn check_banned(&self, stream: &TcpStream) -> bool {
		// peer has been banned, go away!
		if let Ok(peer_addr) = stream.peer_addr() {
			if self.peers.is_banned(peer_addr) {
				debug!(LOGGER, "Peer {} banned, refusing connection.", peer_addr);
				if let Err(e) = stream.shutdown(Shutdown::Both) {
					debug!(LOGGER, "Error shutting down conn: {:?}", e);
				}
				return true;
			}
		}
		false
	}

	pub fn stop(&self) {
		self.stop.store(true, Ordering::Relaxed);
		self.peers.stop();
	}
}

/// A no-op network adapter used for testing.
pub struct DummyAdapter {}

impl ChainAdapter for DummyAdapter {
	fn total_difficulty(&self) -> Difficulty {
		Difficulty::one()
	}
	fn total_height(&self) -> u64 {
		0
	}
	fn transaction_received(&self, _: core::Transaction) {}
	fn compact_block_received(&self, _cb: core::CompactBlock, _addr: SocketAddr) -> bool { true }
	fn header_received(&self, _bh: core::BlockHeader, _addr: SocketAddr) -> bool { true }
	fn block_received(&self, _: core::Block, _: SocketAddr) -> bool { true }
	fn headers_received(&self, _: Vec<core::BlockHeader>, _:SocketAddr) {}
	fn locate_headers(&self, _: Vec<Hash>) -> Vec<core::BlockHeader> {
		vec![]
	}
	fn get_block(&self, _: Hash) -> Option<core::Block> {
		None
	}
	fn sumtrees_read(&self, _h: Hash) -> Option<SumtreesRead> {
		unimplemented!()
	}

	fn sumtrees_write(&self, _h: Hash,
										_rewind_to_output: u64, _rewind_to_kernel: u64,
										_sumtree_data: File, _peer_addr: SocketAddr) -> bool {
		false
	}
}

impl NetAdapter for DummyAdapter {
	fn find_peer_addrs(&self, _: Capabilities) -> Vec<SocketAddr> {
		vec![]
	}
	fn peer_addrs_received(&self, _: Vec<SocketAddr>) {}
	fn peer_difficulty(&self, _: SocketAddr, _: Difficulty, _:u64) {}
}
