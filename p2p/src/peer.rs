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

use crate::util::{Mutex, RwLock};
use std::fmt;
use std::fs::File;
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

use crate::chain;
use crate::conn;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::pow::Difficulty;
use crate::core::{core, global};
use crate::handshake::Handshake;
use crate::msg::{self, BanReason, GetPeerAddrs, Locator, Ping, TxHashSetRequest};
use crate::protocol::Protocol;
use crate::types::{
	Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, PeerAddr, PeerInfo, ReasonForBan,
	TxHashSetRead,
};
use chrono::prelude::{DateTime, Utc};

const MAX_TRACK_SIZE: usize = 30;
const MAX_PEER_MSG_PER_MIN: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Remind: don't mix up this 'State' with that 'State' in p2p/src/store.rs,
///   which has different 3 states: {Healthy, Banned, Defunct}.
///   For example: 'Disconnected' state here could still be 'Healthy' and could reconnect in next loop.
enum State {
	Connected,
	Disconnected,
	Banned,
	// Banned from Peers side, by ban_peer().
	//   This could happen when error in block (or compact block) received, header(s) received,
	//   or txhashset received.
}

pub struct Peer {
	pub info: PeerInfo,
	state: Arc<RwLock<State>>,
	// set of all hashes known to this peer (so no need to send)
	tracking_adapter: TrackingAdapter,
	connection: Option<Mutex<conn::Tracker>>,
}

macro_rules! connection {
	($holder:expr) => {
		match $holder.connection.as_ref() {
			Some(conn) => conn.lock(),
			None => return Err(Error::ConnectionClose),
			}
	};
}

impl fmt::Debug for Peer {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Peer({:?})", &self.info)
	}
}

impl Peer {
	// Only accept and connect can be externally used to build a peer
	fn new(info: PeerInfo, adapter: Arc<dyn NetAdapter>) -> Peer {
		Peer {
			info,
			state: Arc::new(RwLock::new(State::Connected)),
			tracking_adapter: TrackingAdapter::new(adapter),
			connection: None,
		}
	}

	pub fn accept(
		conn: &mut TcpStream,
		capab: Capabilities,
		total_difficulty: Difficulty,
		hs: &Handshake,
		adapter: Arc<dyn NetAdapter>,
	) -> Result<Peer, Error> {
		debug!("accept: handshaking from {:?}", conn.peer_addr());
		let info = hs.accept(capab, total_difficulty, conn);
		match info {
			Ok(peer_info) => Ok(Peer::new(peer_info, adapter)),
			Err(e) => {
				debug!(
					"accept: handshaking from {:?} failed with error: {:?}",
					conn.peer_addr(),
					e
				);
				if let Err(e) = conn.shutdown(Shutdown::Both) {
					debug!("Error shutting down conn: {:?}", e);
				}
				Err(e)
			}
		}
	}

	pub fn connect(
		conn: &mut TcpStream,
		capab: Capabilities,
		total_difficulty: Difficulty,
		self_addr: PeerAddr,
		hs: &Handshake,
		na: Arc<dyn NetAdapter>,
	) -> Result<Peer, Error> {
		debug!("connect: handshaking with {:?}", conn.peer_addr());
		let info = hs.initiate(capab, total_difficulty, self_addr, conn);
		match info {
			Ok(peer_info) => Ok(Peer::new(peer_info, na)),
			Err(e) => {
				debug!(
					"connect: handshaking with {:?} failed with error: {:?}",
					conn.peer_addr(),
					e
				);
				if let Err(e) = conn.shutdown(Shutdown::Both) {
					debug!("Error shutting down conn: {:?}", e);
				}
				Err(e)
			}
		}
	}

	/// Main peer loop listening for messages and forwarding to the rest of the
	/// system.
	pub fn start(&mut self, conn: TcpStream) {
		let adapter = Arc::new(self.tracking_adapter.clone());
		let handler = Protocol::new(adapter, self.info.clone());
		self.connection = Some(Mutex::new(conn::listen(conn, handler)));
	}

	pub fn is_denied(config: &P2PConfig, peer_addr: PeerAddr) -> bool {
		if let Some(ref denied) = config.peers_deny {
			if denied.contains(&peer_addr) {
				debug!(
					"checking peer allowed/denied: {:?} explicitly denied",
					peer_addr
				);
				return true;
			}
		}
		if let Some(ref allowed) = config.peers_allow {
			if allowed.contains(&peer_addr) {
				debug!(
					"checking peer allowed/denied: {:?} explicitly allowed",
					peer_addr
				);
				return false;
			} else {
				debug!(
					"checking peer allowed/denied: {:?} not explicitly allowed, denying",
					peer_addr
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
		self.check_connection()
	}

	/// Whether this peer has been banned.
	pub fn is_banned(&self) -> bool {
		State::Banned == *self.state.read()
	}

	/// Whether this peer is stuck on sync.
	pub fn is_stuck(&self) -> (bool, Difficulty) {
		let peer_live_info = self.info.live_info.read();
		let now = Utc::now().timestamp_millis();
		// if last updated difficulty is 2 hours ago, we're sure this peer is a stuck node.
		if now > peer_live_info.stuck_detector.timestamp_millis() + global::STUCK_PEER_KICK_TIME {
			(true, peer_live_info.total_difficulty)
		} else {
			(false, peer_live_info.total_difficulty)
		}
	}

	/// Whether the peer is considered abusive, mostly for spammy nodes
	pub fn is_abusive(&self) -> bool {
		if let Some(ref conn) = self.connection {
			let conn = conn.lock();
			let rec = conn.received_bytes.read();
			let sent = conn.sent_bytes.read();
			rec.count_per_min() > MAX_PEER_MSG_PER_MIN
				|| sent.count_per_min() > MAX_PEER_MSG_PER_MIN
		} else {
			false
		}
	}

	/// Number of bytes sent to the peer
	pub fn last_min_sent_bytes(&self) -> Option<u64> {
		if let Some(ref tracker) = self.connection {
			let conn = tracker.lock();
			let sent_bytes = conn.sent_bytes.read();
			return Some(sent_bytes.bytes_per_min());
		}
		None
	}

	/// Number of bytes received from the peer
	pub fn last_min_received_bytes(&self) -> Option<u64> {
		if let Some(ref tracker) = self.connection {
			let conn = tracker.lock();
			let received_bytes = conn.received_bytes.read();
			return Some(received_bytes.bytes_per_min());
		}
		None
	}

	pub fn last_min_message_counts(&self) -> Option<(u64, u64)> {
		if let Some(ref tracker) = self.connection {
			let conn = tracker.lock();
			let received_bytes = conn.received_bytes.read();
			let sent_bytes = conn.sent_bytes.read();
			return Some((sent_bytes.count_per_min(), received_bytes.count_per_min()));
		}
		None
	}

	/// Set this peer status to banned
	pub fn set_banned(&self) {
		*self.state.write() = State::Banned;
	}

	/// Send a ping to the remote peer, providing our local difficulty and
	/// height
	pub fn send_ping(&self, total_difficulty: Difficulty, height: u64) -> Result<(), Error> {
		let ping_msg = Ping {
			total_difficulty,
			height,
		};
		connection!(self).send(ping_msg, msg::Type::Ping)
	}

	/// Send the ban reason before banning
	pub fn send_ban_reason(&self, ban_reason: ReasonForBan) -> Result<(), Error> {
		let ban_reason_msg = BanReason { ban_reason };
		connection!(self)
			.send(ban_reason_msg, msg::Type::BanReason)
			.map(|_| ())
	}

	/// Sends the provided block to the remote peer. The request may be dropped
	/// if the remote peer is known to already have the block.
	pub fn send_block(&self, b: &core::Block) -> Result<bool, Error> {
		if !self.tracking_adapter.has_recv(b.hash()) {
			trace!("Send block {} to {}", b.hash(), self.info.addr);
			connection!(self).send(b, msg::Type::Block)?;
			Ok(true)
		} else {
			debug!(
				"Suppress block send {} to {} (already seen)",
				b.hash(),
				self.info.addr,
			);
			Ok(false)
		}
	}

	pub fn send_compact_block(&self, b: &core::CompactBlock) -> Result<bool, Error> {
		if !self.tracking_adapter.has_recv(b.hash()) {
			trace!("Send compact block {} to {}", b.hash(), self.info.addr);
			connection!(self).send(b, msg::Type::CompactBlock)?;
			Ok(true)
		} else {
			debug!(
				"Suppress compact block send {} to {} (already seen)",
				b.hash(),
				self.info.addr,
			);
			Ok(false)
		}
	}

	pub fn send_header(&self, bh: &core::BlockHeader) -> Result<bool, Error> {
		if !self.tracking_adapter.has_recv(bh.hash()) {
			debug!("Send header {} to {}", bh.hash(), self.info.addr);
			connection!(self).send(bh, msg::Type::Header)?;
			Ok(true)
		} else {
			debug!(
				"Suppress header send {} to {} (already seen)",
				bh.hash(),
				self.info.addr,
			);
			Ok(false)
		}
	}

	pub fn send_tx_kernel_hash(&self, h: Hash) -> Result<bool, Error> {
		if !self.tracking_adapter.has_recv(h) {
			debug!("Send tx kernel hash {} to {}", h, self.info.addr);
			connection!(self).send(h, msg::Type::TransactionKernel)?;
			Ok(true)
		} else {
			debug!(
				"Not sending tx kernel hash {} to {} (already seen)",
				h, self.info.addr
			);
			Ok(false)
		}
	}

	/// Sends the provided transaction to the remote peer. The request may be
	/// dropped if the remote peer is known to already have the transaction.
	/// We support broadcast of lightweight tx kernel hash
	/// so track known txs by kernel hash.
	pub fn send_transaction(&self, tx: &core::Transaction) -> Result<bool, Error> {
		let kernel = &tx.kernels()[0];

		if self
			.info
			.capabilities
			.contains(Capabilities::TX_KERNEL_HASH)
		{
			return self.send_tx_kernel_hash(kernel.hash());
		}

		if !self.tracking_adapter.has_recv(kernel.hash()) {
			debug!("Send full tx {} to {}", tx.hash(), self.info.addr);
			connection!(self).send(tx, msg::Type::Transaction)?;
			Ok(true)
		} else {
			debug!(
				"Not sending tx {} to {} (already seen)",
				tx.hash(),
				self.info.addr
			);
			Ok(false)
		}
	}

	/// Sends the provided stem transaction to the remote peer.
	/// Note: tracking adapter is ignored for stem transactions (while under
	/// embargo).
	pub fn send_stem_transaction(&self, tx: &core::Transaction) -> Result<(), Error> {
		debug!("Send (stem) tx {} to {}", tx.hash(), self.info.addr);
		connection!(self).send(tx, msg::Type::StemTransaction)
	}

	/// Sends a request for block headers from the provided block locator
	pub fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error> {
		connection!(self).send(&Locator { hashes: locator }, msg::Type::GetHeaders)
	}

	pub fn send_tx_request(&self, h: Hash) -> Result<(), Error> {
		debug!(
			"Requesting tx (kernel hash) {} from peer {}.",
			h, self.info.addr
		);
		connection!(self).send(&h, msg::Type::GetTransaction)
	}

	/// Sends a request for a specific block by hash
	pub fn send_block_request(&self, h: Hash) -> Result<(), Error> {
		debug!("Requesting block {} from peer {}.", h, self.info.addr);
		self.tracking_adapter.push_req(h);
		connection!(self).send(&h, msg::Type::GetBlock)
	}

	/// Sends a request for a specific compact block by hash
	pub fn send_compact_block_request(&self, h: Hash) -> Result<(), Error> {
		debug!("Requesting compact block {} from {}", h, self.info.addr);
		connection!(self).send(&h, msg::Type::GetCompactBlock)
	}

	pub fn send_peer_request(&self, capab: Capabilities) -> Result<(), Error> {
		trace!("Asking {} for more peers {:?}", self.info.addr, capab);
		connection!(self).send(
			&GetPeerAddrs {
				capabilities: capab,
			},
			msg::Type::GetPeerAddrs,
		)
	}

	pub fn send_txhashset_request(&self, height: u64, hash: Hash) -> Result<(), Error> {
		debug!(
			"Asking {} for txhashset archive at {} {}.",
			self.info.addr, height, hash
		);
		connection!(self).send(
			&TxHashSetRequest { hash, height },
			msg::Type::TxHashSetRequest,
		)
	}

	/// Stops the peer, closing its connection
	pub fn stop(&self) {
		if let Some(conn) = self.connection.as_ref() {
			stop_with_connection(&conn.lock());
		}
	}

	fn check_connection(&self) -> bool {
		let connection = match self.connection.as_ref() {
			Some(conn) => conn.lock(),
			None => return false,
		};
		match connection.error_channel.try_recv() {
			Ok(Error::Serialization(e)) => {
				let need_stop = {
					let mut state = self.state.write();
					if State::Banned != *state {
						*state = State::Disconnected;
						true
					} else {
						false
					}
				};
				if need_stop {
					debug!(
						"Client {} corrupted, will disconnect ({:?}).",
						self.info.addr, e
					);
					stop_with_connection(&connection);
				}
				false
			}
			Ok(e) => {
				let need_stop = {
					let mut state = self.state.write();
					if State::Disconnected != *state {
						*state = State::Disconnected;
						true
					} else {
						false
					}
				};
				if need_stop {
					debug!("Client {} connection lost: {:?}", self.info.addr, e);
					stop_with_connection(&connection);
				}
				false
			}
			Err(_) => {
				let state = self.state.read();
				State::Connected == *state
			}
		}
	}
}

fn stop_with_connection(connection: &conn::Tracker) {
	let _ = connection.close_channel.send(());
}

/// Adapter implementation that forwards everything to an underlying adapter
/// but keeps track of the block and transaction hashes that were requested or
/// received.
#[derive(Clone)]
struct TrackingAdapter {
	adapter: Arc<dyn NetAdapter>,
	known: Arc<RwLock<Vec<Hash>>>,
	requested: Arc<RwLock<Vec<Hash>>>,
}

impl TrackingAdapter {
	fn new(adapter: Arc<dyn NetAdapter>) -> TrackingAdapter {
		TrackingAdapter {
			adapter: adapter,
			known: Arc::new(RwLock::new(Vec::with_capacity(MAX_TRACK_SIZE))),
			requested: Arc::new(RwLock::new(Vec::with_capacity(MAX_TRACK_SIZE))),
		}
	}

	fn has_recv(&self, hash: Hash) -> bool {
		let known = self.known.read();
		// may become too slow, an ordered set (by timestamp for eviction) may
		// end up being a better choice
		known.contains(&hash)
	}

	fn push_recv(&self, hash: Hash) {
		let mut known = self.known.write();
		if known.len() > MAX_TRACK_SIZE {
			known.truncate(MAX_TRACK_SIZE);
		}
		if !known.contains(&hash) {
			known.insert(0, hash);
		}
	}

	fn has_req(&self, hash: Hash) -> bool {
		let requested = self.requested.read();
		// may become too slow, an ordered set (by timestamp for eviction) may
		// end up being a better choice
		requested.contains(&hash)
	}

	fn push_req(&self, hash: Hash) {
		let mut requested = self.requested.write();
		if requested.len() > MAX_TRACK_SIZE {
			requested.truncate(MAX_TRACK_SIZE);
		}
		if !requested.contains(&hash) {
			requested.insert(0, hash);
		}
	}
}

impl ChainAdapter for TrackingAdapter {
	fn total_difficulty(&self) -> Result<Difficulty, chain::Error> {
		self.adapter.total_difficulty()
	}

	fn total_height(&self) -> Result<u64, chain::Error> {
		self.adapter.total_height()
	}

	fn get_transaction(&self, kernel_hash: Hash) -> Option<core::Transaction> {
		self.adapter.get_transaction(kernel_hash)
	}

	fn tx_kernel_received(
		&self,
		kernel_hash: Hash,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		self.push_recv(kernel_hash);
		self.adapter.tx_kernel_received(kernel_hash, peer_info)
	}

	fn transaction_received(
		&self,
		tx: core::Transaction,
		stem: bool,
	) -> Result<bool, chain::Error> {
		// Do not track the tx hash for stem txs.
		// Otherwise we fail to handle the subsequent fluff or embargo expiration
		// correctly.
		if !stem {
			let kernel = &tx.kernels()[0];
			self.push_recv(kernel.hash());
		}
		self.adapter.transaction_received(tx, stem)
	}

	fn block_received(
		&self,
		b: core::Block,
		peer_info: &PeerInfo,
		_was_requested: bool,
	) -> Result<bool, chain::Error> {
		let bh = b.hash();
		self.push_recv(bh);
		self.adapter.block_received(b, peer_info, self.has_req(bh))
	}

	fn compact_block_received(
		&self,
		cb: core::CompactBlock,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		self.push_recv(cb.hash());
		self.adapter.compact_block_received(cb, peer_info)
	}

	fn header_received(
		&self,
		bh: core::BlockHeader,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		self.push_recv(bh.hash());
		self.adapter.header_received(bh, peer_info)
	}

	fn headers_received(
		&self,
		bh: &[core::BlockHeader],
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		self.adapter.headers_received(bh, peer_info)
	}

	fn locate_headers(&self, locator: &[Hash]) -> Result<Vec<core::BlockHeader>, chain::Error> {
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

	fn txhashset_write(
		&self,
		h: Hash,
		txhashset_data: File,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		self.adapter.txhashset_write(h, txhashset_data, peer_info)
	}

	fn txhashset_download_update(
		&self,
		start_time: DateTime<Utc>,
		downloaded_size: u64,
		total_size: u64,
	) -> bool {
		self.adapter
			.txhashset_download_update(start_time, downloaded_size, total_size)
	}

	fn get_tmp_dir(&self) -> PathBuf {
		self.adapter.get_tmp_dir()
	}

	fn get_tmpfile_pathname(&self, tmpfile_name: String) -> PathBuf {
		self.adapter.get_tmpfile_pathname(tmpfile_name)
	}
}

impl NetAdapter for TrackingAdapter {
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<PeerAddr> {
		self.adapter.find_peer_addrs(capab)
	}

	fn peer_addrs_received(&self, addrs: Vec<PeerAddr>) {
		self.adapter.peer_addrs_received(addrs)
	}

	fn peer_difficulty(&self, addr: PeerAddr, diff: Difficulty, height: u64) {
		self.adapter.peer_difficulty(addr, diff, height)
	}

	fn is_banned(&self, addr: PeerAddr) -> bool {
		self.adapter.is_banned(addr)
	}
}
