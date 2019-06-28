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
use std::io::{self, Read};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::chain;
use crate::core::core;
use crate::core::core::hash::Hash;
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::handshake::Handshake;
use crate::peer::Peer;
use crate::peers::Peers;
use crate::store::PeerStore;
use crate::types::{
	Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, PeerAddr, PeerInfo, ReasonForBan,
	Stream, TxHashSetRead,
};
use crate::util::StopState;
use chrono::prelude::{DateTime, Utc};

use i2p::net::{I2pListener, I2pStream};
use i2p::Session;

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	pub config: P2PConfig,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	pub peers: Arc<Peers>,
	i2p_session: Option<Arc<Session>>,
	stop_state: Arc<StopState>,
}

// TODO TLS
impl Server {
	/// Creates a new idle p2p server with no peers
	pub fn new(
		db_root: &str,
		capab: Capabilities,
		config: P2PConfig,
		adapter: Arc<dyn ChainAdapter>,
		i2p_session: Option<Arc<Session>>,
		genesis: Hash,
		stop_state: Arc<StopState>,
	) -> Result<Server, Error> {
		Ok(Server {
			config: config.clone(),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Arc::new(Peers::new(PeerStore::new(db_root)?, adapter, config)),
			i2p_session,
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

		let sleep_time = Duration::from_millis(5);
		loop {
			// Pause peer ingress connection request. Only for tests.
			if self.stop_state.is_paused() {
				thread::sleep(Duration::from_secs(1));
				continue;
			}

			match listener.accept() {
				Ok((stream, peer_addr)) => {
					let peer_addr = PeerAddr::Socket(peer_addr);
					let stream = Stream::Tcp(stream);

					if self.check_undesirable(&stream) {
						continue;
					}
					match self.handle_new_peer(stream) {
						Err(Error::ConnectionClose) => debug!("shutting down, ignoring a new peer"),
						Err(e) => {
							debug!("Error accepting peer {}: {:?}", peer_addr.to_string(), e);
							let _ = self
								.peers
								.add_banned(&peer_addr.clone(), ReasonForBan::BadHandshake);
						}
						Ok(_) => {}
					}
				}
				Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
					// nothing to do, will retry in next iteration
				}
				Err(e) => {
					debug!("Couldn't establish new client connection: {:?}", e);
				}
			}
			if self.stop_state.is_stopped() {
				break;
			}
			thread::sleep(sleep_time);
		}
		Ok(())
	}

	/// Setup I2P streaming listener
	pub fn listen_i2p(&self) -> Result<(), Error> {
		let session = self.i2p_session.as_ref().ok_or(Error::Internal)?;
		let listener = I2pListener::bind_with_session(&session)?;

		for stream in listener.incoming() {
			if self.stop_state.is_stopped() {
				break;
			}
			match stream {
				Ok(stream) => {
					let peer_addr = PeerAddr::I2p(stream.local_addr()?);
					let stream = Stream::I2p(stream);

					if self.check_undesirable(&stream) {
						continue;
					}
					if let Err(e) = self.handle_new_peer(stream) {
						debug!(
							"Error accepting i2p peer {}: {:?}",
							peer_addr.to_string(),
							e
						);
						let _ = self
							.peers
							.add_banned(&peer_addr.clone(), ReasonForBan::BadHandshake);
					}
				}
				Err(e) => {
					debug!("Couldn't establish new i2p connection: {:?}", e);
				}
			}
		}
		// gracefully shutdown listener streams
		if self.stop_state.is_stopped() {
			let _ = self
				.peers
				.connected_peers()
				.iter()
				.filter(|x| x.info.addr.is_i2p())
				.map(|x| x.stop())
				.collect::<Vec<_>>();
		}
		Ok(())
	}

	/// Asks the server to connect to a new peer. Directly returns the peer if
	/// we're already connected to the provided address. Optionally takes an i2p
	/// session to be able to handle i2p client connections.
	pub fn connect(&self, addr: &PeerAddr) -> Result<Arc<Peer>, Error> {
		if self.stop_state.is_stopped() {
			return Err(Error::ConnectionClose);
		}

		if Peer::is_denied(&self.config, &addr.clone()) {
			debug!("connect_peer: peer {} denied, not connecting.", addr);
			return Err(Error::ConnectionClose);
		}

		if global::is_production_mode() {
			let hs = self.handshake.clone();
			let addrs = hs.addrs.read();
			if addrs.contains(&addr.clone()) {
				debug!("connect: ignore connecting to PeerWithSelf, addr: {}", addr);
				return Err(Error::PeerWithSelf);
			}
		}

		if let Some(p) = self.peers.get_connected_peer(&addr.clone()) {
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
		let (stream, self_addr) = match addr {
			PeerAddr::Socket(addr) => {
				let tcp_stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
					.map_err(|e| Error::Connection(e))?;
				let addr = PeerAddr::Socket(SocketAddr::new(self.config.host, self.config.port));
				let stream = Stream::Tcp(tcp_stream);
				(stream, addr)
			}
			PeerAddr::I2p(i2p_addr) => {
				let session = self.i2p_session.as_ref().ok_or(Error::Internal)?;
				let stream =
					Stream::I2p(I2pStream::connect_with_session(&session, i2p_addr.clone())?);
				let addr = stream.local_addr()?;
				(stream, addr)
			}
		};

		let total_diff = self.peers.total_difficulty()?;
		let peer = Peer::connect(
			stream,
			self.capabilities,
			total_diff,
			self_addr,
			&self.handshake,
			self.peers.clone(),
		)?;
		let peer = Arc::new(peer);
		self.peers.add_connected(peer.clone())?;
		Ok(peer)
	}

	fn handle_new_peer(&self, stream: Stream) -> Result<(), Error> {
		let total_diff = self.peers.total_difficulty()?;

		// accept the peer and add it to the server map
		let peer = Peer::accept(
			stream,
			self.capabilities,
			total_diff,
			&self.handshake,
			self.peers.clone(),
		)?;
		self.peers.add_connected(Arc::new(peer))?;
		Ok(())
	}

	/// Checks whether there's any reason we don't want to accept a peer
	/// connection. There can be a couple of them:
	/// 1. The peer has been previously banned and the ban period hasn't
	/// expired yet.
	/// 2. We're already connected to a peer at the same IP. While there are
	/// many reasons multiple peers can legitimately share identical IP
	/// addresses (NAT), network distribution is improved if they choose
	/// different sets of peers themselves. In addition, it prevent potential
	/// duplicate connections, malicious or not.
	fn check_undesirable(&self, stream: &Stream) -> bool {
		if let Ok(peer_addr) = stream.peer_addr() {
			if self.peers.is_banned(&peer_addr.clone()) {
				debug!("Peer {} banned, refusing connection.", peer_addr);
				if let Err(e) = stream.shutdown(Shutdown::Both) {
					debug!("Error shutting down conn: {:?}", e);
				}
				return true;
			}
			if self.peers.is_known(&peer_addr.clone()) {
				debug!("Peer {} already known, refusing connection.", peer_addr);
				if let Err(e) = stream.shutdown(Shutdown::Both) {
					debug!("Error shutting down conn: {:?}", e);
				}
				return true;
			}
		}
		false
	}

	pub fn stop(&self) {
		self.stop_state.stop();
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
	fn total_difficulty(&self) -> Result<Difficulty, chain::Error> {
		Ok(Difficulty::min())
	}
	fn total_height(&self) -> Result<u64, chain::Error> {
		Ok(0)
	}
	fn get_transaction(&self, _h: Hash) -> Option<core::Transaction> {
		None
	}

	fn tx_kernel_received(&self, _h: Hash, _peer_info: &PeerInfo) -> Result<bool, chain::Error> {
		Ok(true)
	}
	fn transaction_received(
		&self,
		_: core::Transaction,
		_stem: bool,
	) -> Result<bool, chain::Error> {
		Ok(true)
	}
	fn compact_block_received(
		&self,
		_cb: core::CompactBlock,
		_peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		Ok(true)
	}
	fn header_received(
		&self,
		_bh: core::BlockHeader,
		_peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		Ok(true)
	}
	fn block_received(&self, _: core::Block, _: &PeerInfo, _: bool) -> Result<bool, chain::Error> {
		Ok(true)
	}
	fn headers_received(
		&self,
		_: &[core::BlockHeader],
		_: &PeerInfo,
	) -> Result<bool, chain::Error> {
		Ok(true)
	}
	fn locate_headers(&self, _: &[Hash]) -> Result<Vec<core::BlockHeader>, chain::Error> {
		Ok(vec![])
	}
	fn get_block(&self, _: Hash) -> Option<core::Block> {
		None
	}
	fn kernel_data_read(&self) -> Result<File, chain::Error> {
		unimplemented!()
	}
	fn kernel_data_write(&self, _reader: &mut Read) -> Result<bool, chain::Error> {
		unimplemented!()
	}
	fn txhashset_read(&self, _h: Hash) -> Option<TxHashSetRead> {
		unimplemented!()
	}

	fn txhashset_archive_header(&self) -> Result<core::BlockHeader, chain::Error> {
		unimplemented!()
	}

	fn txhashset_receive_ready(&self) -> bool {
		false
	}

	fn txhashset_write(
		&self,
		_h: Hash,
		_txhashset_data: File,
		_peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		Ok(false)
	}

	fn txhashset_download_update(
		&self,
		_start_time: DateTime<Utc>,
		_downloaded_size: u64,
		_total_size: u64,
	) -> bool {
		false
	}

	fn get_tmp_dir(&self) -> PathBuf {
		unimplemented!()
	}

	fn get_tmpfile_pathname(&self, _tmpfile_name: String) -> PathBuf {
		unimplemented!()
	}
}

impl NetAdapter for DummyAdapter {
	fn find_peer_addrs(&self, _: Capabilities) -> Vec<PeerAddr> {
		vec![]
	}
	fn peer_addrs_received(&self, _: Vec<PeerAddr>) {}
	fn peer_difficulty(&self, _: &PeerAddr, _: Difficulty, _: u64) {}
	fn is_banned(&self, _: &PeerAddr) -> bool {
		false
	}
}
