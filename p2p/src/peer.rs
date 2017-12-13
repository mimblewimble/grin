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

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use futures::Future;
use tokio_core::net::TcpStream;

use core::core;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use handshake::Handshake;
use types::*;
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
	proto: Box<Protocol>,
	state: Arc<RwLock<State>>,
	// set of all hashes known to this peer (so no need to send)
	tracking_adapter: TrackingAdapter,
}

unsafe impl Sync for Peer {}
unsafe impl Send for Peer {}

impl Peer {
	// Only accept and connect can be externally used to build a peer
	fn new(info: PeerInfo, proto: Box<Protocol>, na: Arc<NetAdapter>) -> Peer {
		Peer {
			info: info,
			proto: proto,
			state: Arc::new(RwLock::new(State::Connected)),
			tracking_adapter: TrackingAdapter::new(na),
		}
	}

	/// Initiates the handshake with another peer.
	pub fn connect(
		conn: TcpStream,
		capab: Capabilities,
		total_difficulty: Difficulty,
		self_addr: SocketAddr,
		hs: Arc<Handshake>,
		na: Arc<NetAdapter>,
	) -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let connect_peer = hs.connect(capab, total_difficulty, self_addr, conn)
			.and_then(|(conn, proto, info)| {
				Ok((conn, Peer::new(info, Box::new(proto), na)))
			});
		Box::new(connect_peer)
	}

	/// Accept a handshake initiated by another peer.
	pub fn accept(
		conn: TcpStream,
		capab: Capabilities,
		total_difficulty: Difficulty,
		hs: &Handshake,
		na: Arc<NetAdapter>,
	) -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let hs_peer = hs.handshake(capab, total_difficulty, conn)
			.and_then(|(conn, proto, info)| {
				Ok((conn, Peer::new(info, Box::new(proto), na)))
			});
		Box::new(hs_peer)
	}

	/// Main peer loop listening for messages and forwarding to the rest of the
	/// system.
	pub fn run(&self, conn: TcpStream) -> Box<Future<Item = (), Error = Error>> {
		let addr = self.info.addr;
		let state = self.state.clone();
		let adapter = Arc::new(self.tracking_adapter.clone());

		Box::new(self.proto.handle(conn, adapter, addr).then(move |res| {
			// handle disconnection, standard disconnections aren't considered an error
			let mut state = state.write().unwrap();
			match res {
				Ok(_) => {
					*state = State::Disconnected;
					info!(LOGGER, "Client {} disconnected.", addr);
					Ok(())
				}
				Err(Error::Serialization(e)) => {
					*state = State::Banned;
					info!(LOGGER, "Client {} corrupted, ban.", addr);
					Err(Error::Serialization(e))
				}
				Err(e) => {
					*state = State::Disconnected;
					info!(LOGGER, "Client {} connection lost: {:?}", addr, e);
					Ok(())
				}
			}
		}))
	}

	/// Whether this peer is still connected.
	pub fn is_connected(&self) -> bool {
		let state = self.state.read().unwrap();
		*state == State::Connected
	}

	/// Whether this peer has been banned.
	pub fn is_banned(&self) -> bool {
		let state = self.state.read().unwrap();
		*state == State::Banned
	}

	/// Set this peer status to banned
	pub fn set_banned(&self) {
		let mut state = self.state.write().unwrap();
		*state = State::Banned;
	}

	/// Bytes sent and received by this peer to the remote peer.
	pub fn transmitted_bytes(&self) -> (u64, u64) {
		self.proto.transmitted_bytes()
	}

	pub fn send_ping(&self, total_difficulty: Difficulty) -> Result<(), Error> {
		self.proto.send_ping(total_difficulty)
	}

	/// Sends the provided block to the remote peer. The request may be dropped
	/// if the remote peer is known to already have the block.
	pub fn send_block(&self, b: &core::Block) -> Result<(), Error> {
		if !self.tracking_adapter.has(b.hash()) {
			self.proto.send_block(b)
		} else {
			Ok(())
		}
	}

	/// Sends the provided transaction to the remote peer. The request may be
	/// dropped if the remote peer is known to already have the transaction.
	pub fn send_transaction(&self, tx: &core::Transaction) -> Result<(), Error> {
		if !self.tracking_adapter.has(tx.hash()) {
			self.proto.send_transaction(tx)
		} else {
			Ok(())
		}
	}

	pub fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error> {
		self.proto.send_header_request(locator)
	}

	pub fn send_block_request(&self, h: Hash) -> Result<(), Error> {
		debug!(
			LOGGER,
			"Requesting block {} from peer {}.",
			h,
			self.info.addr
		);
		self.proto.send_block_request(h)
	}

	pub fn send_peer_request(&self, capab: Capabilities) -> Result<(), Error> {
		debug!(LOGGER, "Asking {} for more peers.", self.info.addr);
		self.proto.send_peer_request(capab)
	}

	pub fn stop(&self) {
		self.proto.close();
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

impl NetAdapter for TrackingAdapter {
	fn total_difficulty(&self) -> Difficulty {
		self.adapter.total_difficulty()
	}

	fn transaction_received(&self, tx: core::Transaction) {
		self.push(tx.hash());
		self.adapter.transaction_received(tx)
	}

	fn block_received(&self, b: core::Block, addr: SocketAddr) {
		self.push(b.hash());
		self.adapter.block_received(b, addr)
	}

	fn headers_received(&self, bh: Vec<core::BlockHeader>, addr: SocketAddr) {
		self.adapter.headers_received(bh, addr)
	}

	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader> {
		self.adapter.locate_headers(locator)
	}

	fn get_block(&self, h: Hash) -> Option<core::Block> {
		self.adapter.get_block(h)
	}

	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<SocketAddr> {
		self.adapter.find_peer_addrs(capab)
	}

	fn peer_addrs_received(&self, addrs: Vec<SocketAddr>) {
		self.adapter.peer_addrs_received(addrs)
	}

	fn peer_connected(&self, pi: &PeerInfo) {
		self.adapter.peer_connected(pi)
	}

	fn peer_difficulty(&self, addr: SocketAddr, diff: Difficulty) {
		self.adapter.peer_difficulty(addr, diff)
	}
}
