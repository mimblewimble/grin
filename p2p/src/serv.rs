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

use std::fs::File;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::{io, thread};

use lmdb;

use core::core;
use core::core::hash::Hash;
use core::pow::Difficulty;
use handshake::Handshake;
use peer::Peer;
use peers::Peers;
use store::PeerStore;
use types::{Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, TxHashSetRead};
use util::LOGGER;

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	pub config: P2PConfig,
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
		db_env: Arc<lmdb::Environment>,
		mut capab: Capabilities,
		config: P2PConfig,
		adapter: Arc<ChainAdapter>,
		genesis: Hash,
		stop: Arc<AtomicBool>,
		_archive_mode: bool,
		block_1_hash: Option<Hash>,
	) -> Result<Server, Error> {
		// In the case of an archive node, check that we do have the first block.
		// In case of first sync we do not perform this check.
		if capab.contains(Capabilities::FULL_HIST) && adapter.total_height() > 0 {
			// Check that we have block 1
			match block_1_hash {
				Some(hash) => match adapter.get_block(hash) {
					Some(_) => debug!(LOGGER, "Full block 1 found, archive capabilities confirmed"),
					None => {
						debug!(
							LOGGER,
							"Full block 1 not found, archive capabilities disabled"
						);
						capab.remove(Capabilities::FULL_HIST);
					}
				},
				None => {
					debug!(LOGGER, "Block 1 not found, archive capabilities disabled");
					capab.remove(Capabilities::FULL_HIST);
				}
			}
		}
		Ok(Server {
			config: config.clone(),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Arc::new(Peers::new(PeerStore::new(db_env)?, adapter, config)),
			stop: stop,
		})
	}

	/// Starts a new TCP server and listen to incoming connections. This is a
	/// blocking call until the TCP server stops.
	pub fn listen(&self) -> Result<(), Error> {
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
							warn!(
								LOGGER,
								"Error accepting peer {}: {:?}",
								peer_addr.to_string(),
								e
							);
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
			debug!(
				LOGGER,
				"connect_peer: peer {} denied, not connecting.", addr
			);
			return Err(Error::ConnectionClose);
		}

		// check ip and port to see if we are trying to connect to ourselves
		// todo: this can't detect all cases of PeerWithSelf, for example config.host is '0.0.0.0'
		//
		if self.config.port == addr.port()
			&& (addr.ip().is_loopback() || addr.ip() == self.config.host)
		{
			return Err(Error::PeerWithSelf);
		}

		if let Some(p) = self.peers.get_connected_peer(addr) {
			// if we're already connected to the addr, just return the peer
			trace!(LOGGER, "connect_peer: already connected {}", addr);
			return Ok(p);
		}

		trace!(
			LOGGER,
			"connect_peer: on {}:{}. connecting to {}",
			self.config.host,
			self.config.port,
			addr
		);
		match TcpStream::connect_timeout(addr, Duration::from_secs(10)) {
			Ok(mut stream) => {
				let addr = SocketAddr::new(self.config.host, self.config.port);
				let total_diff = self.peers.total_difficulty();

				let peer = Arc::new(RwLock::new(Peer::connect(
					&mut stream,
					self.capabilities,
					total_diff,
					addr,
					&self.handshake,
					self.peers.clone(),
				)?));
				{
					let mut peer = peer.write().unwrap();
					peer.start(stream);
				}
				self.peers.add_connected(peer.clone())?;
				Ok(peer)
			}
			Err(e) => {
				debug!(
					LOGGER,
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
		let peer = Arc::new(RwLock::new(Peer::accept(
			&mut stream,
			self.capabilities,
			total_diff,
			&self.handshake,
			self.peers.clone(),
		)?));
		{
			let mut peer = peer.write().unwrap();
			peer.start(stream);
		}
		self.peers.add_connected(peer)?;
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
	fn headers_received(&self, _: Vec<core::BlockHeader>, _: SocketAddr) -> bool {
		true
	}
	fn locate_headers(&self, _: Vec<Hash>) -> Vec<core::BlockHeader> {
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
