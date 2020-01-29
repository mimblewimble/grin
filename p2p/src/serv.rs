// Copyright 2020 The Grin Developers
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

use futures::channel::oneshot;
use futures::prelude::*;
use futures::stream::select;
use std::fs::File;
use std::io::Read;
use std::net::{Shutdown, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

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
	TxHashSetRead,
};
use crate::util::StopState;
use chrono::prelude::{DateTime, Utc};

type StopTx = oneshot::Sender<()>;
type StopRx = oneshot::Receiver<()>;

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	pub config: P2PConfig,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	pub peers: Arc<Peers>,
	stop_state: Arc<StopState>,
	stop_tx: StopTx,
}

enum Listen {
	Stream(Result<tokio::net::TcpStream, std::io::Error>),
	Stop,
}

// TODO TLS
impl Server {
	/// Creates a new idle p2p server with no peers
	pub fn new(
		db_root: &str,
		capab: Capabilities,
		config: P2PConfig,
		adapter: Arc<dyn ChainAdapter>,
		genesis: Hash,
		stop_state: Arc<StopState>,
		stop_tx: StopTx,
	) -> Result<Server, Error> {
		Ok(Server {
			config: config.clone(),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Arc::new(Peers::new(PeerStore::new(db_root)?, adapter, config)),
			stop_state,
			stop_tx,
		})
	}

	/// Asks the server to connect to a new peer. Directly returns the peer if
	/// we're already connected to the provided address.
	pub async fn connect(&self, addr: PeerAddr) -> Result<Arc<Peer>, Error> {
		if self.stop_state.is_stopped() {
			return Err(Error::ConnectionClose);
		}

		if Peer::is_denied(&self.config, addr) {
			debug!("connect_peer: peer {} denied, not connecting.", addr);
			return Err(Error::ConnectionClose);
		}

		if global::is_production_mode() {
			let hs = self.handshake.clone();
			let addrs = hs.addrs.read().await;
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
		match TcpStream::connect(&addr.0).await {
			Ok(stream) => {
				let addr = SocketAddr::new(self.config.host, self.config.port);
				let total_diff = self.peers.total_difficulty()?;

				let peer = Peer::connect(
					stream,
					self.capabilities,
					total_diff,
					PeerAddr(addr),
					&self.handshake,
					self.peers.clone(),
				)
				.await?;
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

	async fn handle_new_peer(&self, stream: tokio::net::TcpStream) -> Result<(), Error> {
		let peer_addr = PeerAddr(stream.peer_addr()?);
		if self.check_undesirable(peer_addr) {
			// TODO: async
			// Shutdown the incoming TCP connection if it is not desired
			if let Err(e) = stream.shutdown(Shutdown::Both) {
				debug!("Error shutting down conn: {:?}", e);
			}
			return Ok(());
		}

		if self.stop_state.is_stopped() {
			return Err(Error::ConnectionClose);
		}
		let total_diff = self.peers.total_difficulty()?;

		// accept the peer and add it to the server map
		let peer = Peer::accept(
			stream,
			self.capabilities,
			total_diff,
			&self.handshake,
			self.peers.clone(),
		)
		.await?;
		let peers_inner = self.peers.clone();
		tokio::task::spawn_blocking(move || peers_inner.add_connected(Arc::new(peer)))
			.await
			.unwrap()?;
		Ok(())
	}

	/// Checks whether there's any reason we don't want to accept an incoming peer
	/// connection. There can be a few of them:
	/// 1. Accepting the peer connection would exceed the configured maximum allowed
	/// inbound peer count. Note that seed nodes may wish to increase the default
	/// value for PEER_LISTENER_BUFFER_COUNT to help with network bootstrapping.
	/// A default buffer of 8 peers is allowed to help with network growth.
	/// 2. The peer has been previously banned and the ban period hasn't
	/// expired yet.
	/// 3. We're already connected to a peer at the same IP. While there are
	/// many reasons multiple peers can legitimately share identical IP
	/// addresses (NAT), network distribution is improved if they choose
	/// different sets of peers themselves. In addition, it prevent potential
	/// duplicate connections, malicious or not.
	fn check_undesirable(&self, peer_addr: PeerAddr) -> bool {
		if self.peers.peer_inbound_count()
			>= self.config.peer_max_inbound_count() + self.config.peer_listener_buffer_count()
		{
			debug!("Accepting new connection will exceed peer limit, refusing connection.");
			return true;
		}

		if self.peers.is_banned(peer_addr) {
			debug!("Peer {} banned, refusing connection.", peer_addr);
			return true;
		}

		// The call to is_known() can fail due to contention on the peers map.
		// If it fails we want to default to refusing the connection.
		match self.peers.is_known(peer_addr) {
			Ok(true) => {
				debug!("Peer {} already known, refusing connection.", peer_addr);
				true
			}
			Err(_) => {
				error!(
					"Peer {} is_known check failed, refusing connection.",
					peer_addr
				);
				true
			}
			_ => false,
		}
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
	fn block_received(
		&self,
		_: core::Block,
		_: &PeerInfo,
		_: chain::Options,
	) -> Result<bool, chain::Error> {
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
	fn kernel_data_write(&self, _reader: &mut dyn Read) -> Result<bool, chain::Error> {
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
	fn peer_difficulty(&self, _: PeerAddr, _: Difficulty, _: u64) {}
	fn is_banned(&self, _: PeerAddr) -> bool {
		false
	}
}

/// Starts a new TCP server and listen to incoming connections
pub async fn listen(server: Arc<Server>, stop_rx: StopRx) -> Result<(), Error> {
	// start TCP listener and handle incoming connections
	let addr = SocketAddr::new(server.config.host, server.config.port);
	let mut listener = TcpListener::bind(addr).await?;
	let incoming = listener.incoming().map(|s| Listen::Stream(s));
	let stop = stop_rx.into_stream().map(|_| Listen::Stop);
	let mut select = select(incoming, stop);

	while let Some(next) = select.next().await {
		match next {
			Listen::Stream(Ok(stream)) => {
				// Spawn a new task to handle the incoming connection
				let server_inner = server.clone();
				tokio::spawn(async move {
					let server = server_inner;
					let peer_addr = match stream.peer_addr() {
						Ok(a) => PeerAddr(a),
						Err(_) => return,
					};

					if let Err(e) = server.handle_new_peer(stream).await {
						debug!("Error accepting peer {}: {:?}", peer_addr.to_string(), e);
						let _ = server
							.peers
							.add_banned(peer_addr, ReasonForBan::BadHandshake);
					}
				});
			}
			Listen::Stream(Err(e)) => debug!("Error accepting peer: {:?}", e),
			Listen::Stop => break,
		}
	}

	Ok(())
}
