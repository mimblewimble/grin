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
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, RwLock};

use conn;
use core::core;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use handshake::Handshake;
use msg::{self, BanReason, GetPeerAddrs, Locator, Ping, TxHashSetRequest};
use protocol::Protocol;
use types::{
	Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, PeerInfo, ReasonForBan, TxHashSetRead,
};
use util::LOGGER;

const MAX_TRACK_SIZE: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
	Connected,
	Disconnected,
	Banned,
}

pub struct Peer {
	pub info: PeerInfo,
	state: Arc<RwLock<State>>,
	// set of all hashes known to this peer (so no need to send)
	tracking_adapter: TrackingAdapter,
	connection: Option<conn::Tracker>,
}

unsafe impl Sync for Peer {}
unsafe impl Send for Peer {}

impl Peer {
	// Only accept and connect can be externally used to build a peer
	fn new(info: PeerInfo, na: Arc<NetAdapter>) -> Peer {
		Peer {
			info: info,
			state: Arc::new(RwLock::new(State::Connected)),
			tracking_adapter: TrackingAdapter::new(na),
			connection: None,
		}
	}

	pub fn accept(
		conn: &mut TcpStream,
		capab: Capabilities,
		total_difficulty: Difficulty,
		hs: &Handshake,
		na: Arc<NetAdapter>,
	) -> Result<Peer, Error> {
		let info = hs.accept(capab, total_difficulty, conn)?;
		Ok(Peer::new(info, na))
	}

	pub fn connect(
		conn: &mut TcpStream,
		capab: Capabilities,
		total_difficulty: Difficulty,
		self_addr: SocketAddr,
		hs: &Handshake,
		na: Arc<NetAdapter>,
	) -> Result<Peer, Error> {
		let info = hs.initiate(capab, total_difficulty, self_addr, conn)?;
		Ok(Peer::new(info, na))
	}

	/// Main peer loop listening for messages and forwarding to the rest of the
	/// system.
	pub fn start(&mut self, conn: TcpStream) {
		let addr = self.info.addr;
		let adapter = Arc::new(self.tracking_adapter.clone());
		let handler = Protocol::new(adapter, addr);
		self.connection = Some(conn::listen(conn, handler));
	}

	pub fn is_denied(config: &P2PConfig, peer_addr: &SocketAddr) -> bool {
		let peer = format!("{}:{}", peer_addr.ip(), peer_addr.port());
		if let Some(ref denied) = config.peers_deny {
			if denied.contains(&peer) {
				debug!(
					LOGGER,
					"checking peer allowed/denied: {:?} explicitly denied", peer_addr
				);
				return true;
			}
		}
		if let Some(ref allowed) = config.peers_allow {
			if allowed.contains(&peer) {
				debug!(
					LOGGER,
					"checking peer allowed/denied: {:?} explicitly allowed", peer_addr
				);
				return false;
			} else {
				debug!(
					LOGGER,
					"checking peer allowed/denied: {:?} not explicitly allowed, denying", peer_addr
				);
				return true;
			}
		}

		// default to allowing peer connection if we do not explicitly allow or deny
		// the peer
		false
	}

	/// Whether this peer is still connected.
	pub fn is_connected(&self) -> bool {
		if !self.check_connection() {
			return false;
		}
		let state = self.state.read().unwrap();
		*state == State::Connected
	}

	/// Whether this peer has been banned.
	pub fn is_banned(&self) -> bool {
		let _ = self.check_connection();
		let state = self.state.read().unwrap();
		*state == State::Banned
	}

	/// Set this peer status to banned
	pub fn set_banned(&self) {
		let mut state = self.state.write().unwrap();
		*state = State::Banned;
	}

	/// Send a ping to the remote peer, providing our local difficulty and
	/// height
	pub fn send_ping(&self, total_difficulty: Difficulty, height: u64) -> Result<(), Error> {
		let ping_msg = Ping {
			total_difficulty,
			height,
		};
		self.connection
			.as_ref()
			.unwrap()
			.send(ping_msg, msg::Type::Ping)
	}

	/// Send the ban reason before banning
	pub fn send_ban_reason(&self, ban_reason: ReasonForBan) {
		let ban_reason_msg = BanReason { ban_reason };
		match self
			.connection
			.as_ref()
			.unwrap()
			.send(ban_reason_msg, msg::Type::BanReason)
		{
			Ok(_) => debug!(
				LOGGER,
				"Sent ban reason {:?} to {}", ban_reason, self.info.addr
			),
			Err(e) => error!(
				LOGGER,
				"Could not send ban reason {:?} to {}: {:?}", ban_reason, self.info.addr, e
			),
		};
	}

	/// Sends the provided block to the remote peer. The request may be dropped
	/// if the remote peer is known to already have the block.
	pub fn send_block(&self, b: &core::Block) -> Result<(), Error> {
		if !self.tracking_adapter.has(b.hash()) {
			trace!(LOGGER, "Send block {} to {}", b.hash(), self.info.addr);
			self.connection.as_ref().unwrap().send(b, msg::Type::Block)
		} else {
			debug!(
				LOGGER,
				"Suppress block send {} to {} (already seen)",
				b.hash(),
				self.info.addr,
			);
			Ok(())
		}
	}

	pub fn send_compact_block(&self, b: &core::CompactBlock) -> Result<(), Error> {
		if !self.tracking_adapter.has(b.hash()) {
			trace!(
				LOGGER,
				"Send compact block {} to {}",
				b.hash(),
				self.info.addr
			);
			self.connection
				.as_ref()
				.unwrap()
				.send(b, msg::Type::CompactBlock)
		} else {
			debug!(
				LOGGER,
				"Suppress compact block send {} to {} (already seen)",
				b.hash(),
				self.info.addr,
			);
			Ok(())
		}
	}

	pub fn send_header(&self, bh: &core::BlockHeader) -> Result<(), Error> {
		if !self.tracking_adapter.has(bh.hash()) {
			debug!(LOGGER, "Send header {} to {}", bh.hash(), self.info.addr);
			self.connection
				.as_ref()
				.unwrap()
				.send(bh, msg::Type::Header)
		} else {
			trace!(
				LOGGER,
				"Suppress header send {} to {} (already seen)",
				bh.hash(),
				self.info.addr,
			);
			Ok(())
		}
	}

	/// Sends the provided transaction to the remote peer. The request may be
	/// dropped if the remote peer is known to already have the transaction.
	pub fn send_transaction(&self, tx: &core::Transaction) -> Result<(), Error> {
		if !self.tracking_adapter.has(tx.hash()) {
			debug!(LOGGER, "Send tx {} to {}", tx.hash(), self.info.addr);
			self.connection
				.as_ref()
				.unwrap()
				.send(tx, msg::Type::Transaction)
		} else {
			debug!(
				LOGGER,
				"Not sending tx {} to {} (already seen)",
				tx.hash(),
				self.info.addr
			);
			Ok(())
		}
	}

	/// Sends the provided stem transaction to the remote peer.
	/// Note: tracking adapter is ignored for stem transactions (while under
	/// embargo).
	pub fn send_stem_transaction(&self, tx: &core::Transaction) -> Result<(), Error> {
		debug!(LOGGER, "Send (stem) tx {} to {}", tx.hash(), self.info.addr);
		self.connection
			.as_ref()
			.unwrap()
			.send(tx, msg::Type::StemTransaction)?;
		Ok(())
	}

	/// Sends a request for block headers from the provided block locator
	pub fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error> {
		self.connection
			.as_ref()
			.unwrap()
			.send(&Locator { hashes: locator }, msg::Type::GetHeaders)
	}

	/// Sends a request for a specific block by hash
	pub fn send_block_request(&self, h: Hash) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Requesting block {} from peer {}.", h, self.info.addr
		);
		self.connection
			.as_ref()
			.unwrap()
			.send(&h, msg::Type::GetBlock)
	}

	/// Sends a request for a specific compact block by hash
	pub fn send_compact_block_request(&self, h: Hash) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Requesting compact block {} from {}", h, self.info.addr
		);
		self.connection
			.as_ref()
			.unwrap()
			.send(&h, msg::Type::GetCompactBlock)
	}

	pub fn send_peer_request(&self, capab: Capabilities) -> Result<(), Error> {
		debug!(LOGGER, "Asking {} for more peers.", self.info.addr);
		self.connection.as_ref().unwrap().send(
			&GetPeerAddrs {
				capabilities: capab,
			},
			msg::Type::GetPeerAddrs,
		)
	}

	pub fn send_txhashset_request(&self, height: u64, hash: Hash) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Asking {} for txhashset archive at {} {}.", self.info.addr, height, hash
		);
		self.connection.as_ref().unwrap().send(
			&TxHashSetRequest { hash, height },
			msg::Type::TxHashSetRequest,
		)
	}

	/// Stops the peer, closing its connection
	pub fn stop(&self) {
		let _ = self.connection.as_ref().unwrap().close_channel.send(());
	}

	fn check_connection(&self) -> bool {
		match self.connection.as_ref().unwrap().error_channel.try_recv() {
			Ok(Error::Serialization(e)) => {
				let mut state = self.state.write().unwrap();
				*state = State::Banned;
				info!(
					LOGGER,
					"Client {} corrupted, ban ({:?}).", self.info.addr, e
				);
				false
			}
			Ok(e) => {
				let mut state = self.state.write().unwrap();
				*state = State::Disconnected;
				debug!(LOGGER, "Client {} connection lost: {:?}", self.info.addr, e);
				false
			}
			Err(_) => true,
		}
	}
}

/// Adapter implementation that forwards everything to an underlying adapter
/// but keeps track of the block and transaction hashes that were received.
#[derive(Clone)]
struct TrackingAdapter {
	adapter: Arc<NetAdapter>,
	known: Arc<RwLock<Vec<Hash>>>,
}

impl TrackingAdapter {
	fn new(adapter: Arc<NetAdapter>) -> TrackingAdapter {
		TrackingAdapter {
			adapter: adapter,
			known: Arc::new(RwLock::new(vec![])),
		}
	}

	fn has(&self, hash: Hash) -> bool {
		let known = self.known.read().unwrap();
		// may become too slow, an ordered set (by timestamp for eviction) may
		// end up being a better choice
		known.contains(&hash)
	}

	fn push(&self, hash: Hash) {
		let mut known = self.known.write().unwrap();
		if known.len() > MAX_TRACK_SIZE {
			known.truncate(MAX_TRACK_SIZE);
		}
		known.insert(0, hash);
	}
}

impl ChainAdapter for TrackingAdapter {
	fn total_difficulty(&self) -> Difficulty {
		self.adapter.total_difficulty()
	}

	fn total_height(&self) -> u64 {
		self.adapter.total_height()
	}

	fn transaction_received(&self, tx: core::Transaction, stem: bool) {
		// Do not track the tx hash for stem txs.
		// Otherwise we fail to handle the subsequent fluff or embargo expiration
		// correctly.
		if !stem {
			self.push(tx.hash());
		}
		self.adapter.transaction_received(tx, stem)
	}

	fn block_received(&self, b: core::Block, addr: SocketAddr) -> bool {
		self.push(b.hash());
		self.adapter.block_received(b, addr)
	}

	fn compact_block_received(&self, cb: core::CompactBlock, addr: SocketAddr) -> bool {
		self.push(cb.hash());
		self.adapter.compact_block_received(cb, addr)
	}

	fn header_received(&self, bh: core::BlockHeader, addr: SocketAddr) -> bool {
		self.push(bh.hash());
		self.adapter.header_received(bh, addr)
	}

	fn headers_received(&self, bh: Vec<core::BlockHeader>, addr: SocketAddr) -> bool {
		self.adapter.headers_received(bh, addr)
	}

	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader> {
		self.adapter.locate_headers(locator)
	}

	fn get_block(&self, h: Hash) -> Option<core::Block> {
		self.adapter.get_block(h)
	}

	fn txhashset_read(&self, h: Hash) -> Option<TxHashSetRead> {
		self.adapter.txhashset_read(h)
	}

	fn txhashset_receive_ready(&self) -> bool {
		self.adapter.txhashset_receive_ready()
	}

	fn txhashset_write(&self, h: Hash, txhashset_data: File, peer_addr: SocketAddr) -> bool {
		self.adapter.txhashset_write(h, txhashset_data, peer_addr)
	}
}

impl NetAdapter for TrackingAdapter {
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<SocketAddr> {
		self.adapter.find_peer_addrs(capab)
	}

	fn peer_addrs_received(&self, addrs: Vec<SocketAddr>) {
		self.adapter.peer_addrs_received(addrs)
	}

	fn peer_difficulty(&self, addr: SocketAddr, diff: Difficulty, height: u64) {
		self.adapter.peer_difficulty(addr, diff, height)
	}

	fn is_banned(&self, addr: SocketAddr) -> bool {
		self.adapter.is_banned(addr)
	}
}
