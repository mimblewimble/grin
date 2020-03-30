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
use crate::util::{Mutex, StopState};
use crate::State;
use chrono::prelude::{DateTime, Utc};
use futures::channel::oneshot;
use futures::prelude::*;
use std::fs::File;
use std::io::Read;
use std::net::{Shutdown, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	pub config: P2PConfig,
	pub runtime: Arc<Runtime>,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	pub peers: Arc<Peers>,
	stop_state: Arc<StopState>,
	stop_tx: Mutex<Option<oneshot::Sender<()>>>,
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
		stop_tx: oneshot::Sender<()>,
	) -> Result<Server, Error> {
		let runtime = tokio::runtime::Builder::new()
			.threaded_scheduler()
			.enable_all()
			.build()?;

		Ok(Server {
			config: config.clone(),
			runtime: Arc::new(runtime),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Arc::new(Peers::new(PeerStore::new(db_root)?, adapter, config)),
			stop_state,
			stop_tx: Mutex::new(Some(stop_tx)),
		})
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
		if let Some(stop_tx) = self.stop_tx.lock().take() {
			let _ = stop_tx.send(());
		}
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

async fn handle_new_peer(server: &Server, stream: tokio::net::TcpStream) -> Result<(), Error> {
	let peer_addr = PeerAddr(stream.peer_addr()?);
	if server.check_undesirable(peer_addr) {
		// TODO: async
		// Shutdown the incoming TCP connection if it is not desired
		if let Err(e) = stream.shutdown(Shutdown::Both) {
			debug!("Error shutting down conn: {:?}", e);
		}
		return Ok(());
	}

	if server.stop_state.is_stopped() {
		return Err(Error::ConnectionClose);
	}
	let total_diff = server.peers.total_difficulty()?;

	// accept the peer and add it to the server map
	let peer = Peer::accept(
		stream,
		server.capabilities,
		total_diff,
		&server.handshake,
		server.peers.clone(),
	)
	.await?;

	server.peers.add_connected(Arc::new(peer))?;
	Ok(())
}

/// Starts a new TCP server and listen to incoming connections
pub async fn listen(server: Arc<Server>) -> Result<(), Error> {
	let addr = SocketAddr::new(server.config.host, server.config.port);
	let mut listener = TcpListener::bind(addr).await?;
	let mut incoming = listener.incoming();

	while let Some(stream) = incoming.next().await.transpose()? {
		// Spawn a new task to handle the incoming connection
		let server_inner = server.clone();
		tokio::spawn(async move {
			let server = server_inner;
			let peer_addr = match stream.peer_addr() {
				Ok(a) => PeerAddr(a),
				Err(_) => return,
			};

			if let Err(e) = handle_new_peer(&server, stream).await {
				debug!("Error accepting peer {}: {:?}", peer_addr.to_string(), e);
				let _ = server
					.peers
					.add_banned(peer_addr, ReasonForBan::BadHandshake);
			}
		});
	}

	Ok(())
}

async fn connect_internal(server: &Server, addr: PeerAddr) -> Result<Arc<Peer>, Error> {
	if server.stop_state.is_stopped() {
		return Err(Error::ConnectionClose);
	}

	if Peer::is_denied(&server.config, addr) {
		debug!("connect_peer: peer {} denied, not connecting.", addr);
		return Err(Error::ConnectionClose);
	}

	if global::is_production_mode() {
		let hs = server.handshake.clone();
		let addrs = hs.addrs.read().await;
		if addrs.contains(&addr) {
			debug!("connect: ignore connecting to PeerWithSelf, addr: {}", addr);
			return Err(Error::PeerWithSelf);
		}
	}

	if let Some(p) = server.peers.get_connected_peer(addr) {
		// if we're already connected to the addr, just return the peer
		trace!("connect_peer: already connected {}", addr);
		return Ok(p);
	}

	trace!(
		"connect_peer: on {}:{}. connecting to {}",
		server.config.host,
		server.config.port,
		addr
	);
	match TcpStream::connect(&addr.0).await {
		Ok(stream) => {
			let addr = SocketAddr::new(server.config.host, server.config.port);
			let total_diff = server.peers.total_difficulty()?;

			let peer = Peer::connect(
				stream,
				server.capabilities,
				total_diff,
				PeerAddr(addr),
				&server.handshake,
				server.peers.clone(),
			)
			.await?;
			let peer = Arc::new(peer);
			server.peers.add_connected(peer.clone())?;
			Ok(peer)
		}
		Err(e) => {
			trace!(
				"connect_peer: on {}:{}. Could not connect to {}: {:?}",
				server.config.host,
				server.config.port,
				addr,
				e
			);
			Err(Error::Connection(e))
		}
	}
}

/// Attempt to connect to a peer
pub async fn connect(server: Arc<Server>, addr: PeerAddr) {
	match connect_internal(&server, addr).await {
		Ok(peer) => {
			if peer.send_peer_request(server.capabilities.clone()).is_ok() {
				let _ = server.peers.update_state(addr, State::Healthy);
			}
		}
		Err(_) => {
			let _ = server.peers.update_state(addr, State::Defunct);
		}
	}
}
