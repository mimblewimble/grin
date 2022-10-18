// Copyright 2021 The Grin Developers
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

use crate::util::RwLock;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use rand::prelude::*;

use crate::chain;
use crate::chain::txhashset::BitmapChunk;
use crate::core::core;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::{OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::msg::PeerAddrs;
use crate::peer::Peer;
use crate::store::{PeerData, PeerStore, State};
use crate::types::{
	Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, PeerAddr, PeerInfo, ReasonForBan,
	TxHashSetRead, MAX_PEER_ADDRS,
};
use crate::util::secp::pedersen::RangeProof;
use chrono::prelude::*;
use chrono::Duration;

const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub struct Peers {
	pub adapter: Arc<dyn ChainAdapter>,
	store: PeerStore,
	peers: RwLock<HashMap<PeerAddr, Arc<Peer>>>,
	config: P2PConfig,
}

impl Peers {
	pub fn new(store: PeerStore, adapter: Arc<dyn ChainAdapter>, config: P2PConfig) -> Peers {
		Peers {
			adapter,
			store,
			config,
			peers: RwLock::new(HashMap::new()),
		}
	}

	/// Adds the peer to our internal peer mapping. Note that the peer is still
	/// returned so the server can run it.
	pub fn add_connected(&self, peer: Arc<Peer>) -> Result<(), Error> {
		let peer_data: PeerData;
		{
			// Scope for peers vector lock - dont hold the peers lock while adding to lmdb
			let mut peers = self.peers.try_write_for(LOCK_TIMEOUT).ok_or_else(|| {
				error!("add_connected: failed to get peers lock");
				Error::Timeout
			})?;
			peer_data = PeerData {
				addr: peer.info.addr,
				capabilities: peer.info.capabilities,
				user_agent: peer.info.user_agent.clone(),
				flags: State::Healthy,
				last_banned: 0,
				ban_reason: ReasonForBan::None,
				last_connected: Utc::now().timestamp(),
			};
			debug!("Adding newly connected peer {}.", peer_data.addr);
			peers.insert(peer_data.addr, peer);
		}
		debug!("Saving newly connected peer {}.", peer_data.addr);
		if let Err(e) = self.save_peer(&peer_data) {
			error!("Could not save connected peer address: {:?}", e);
		}
		Ok(())
	}

	/// Add a peer as banned to block future connections, usually due to failed
	/// handshake
	pub fn add_banned(&self, addr: PeerAddr, ban_reason: ReasonForBan) -> Result<(), Error> {
		let peer_data = PeerData {
			addr,
			capabilities: Capabilities::UNKNOWN,
			user_agent: "".to_string(),
			flags: State::Banned,
			last_banned: Utc::now().timestamp(),
			ban_reason,
			last_connected: Utc::now().timestamp(),
		};
		debug!("Banning peer {}.", addr);
		self.save_peer(&peer_data)
	}

	/// Check if this peer address is already known (are we already connected to it)?
	/// We try to get the read lock but if we experience contention
	/// and this attempt fails then return an error allowing the caller
	/// to decide how best to handle this.
	pub fn is_known(&self, addr: PeerAddr) -> Result<bool, Error> {
		let peers = self.peers.try_read_for(LOCK_TIMEOUT).ok_or_else(|| {
			error!("is_known: failed to get peers lock");
			Error::Internal
		})?;
		Ok(peers.contains_key(&addr))
	}

	/// Iterator over our current peers.
	/// This allows us to hide try_read_for() behind a cleaner interface.
	/// PeersIter lets us chain various adaptors for convenience.
	pub fn iter(&self) -> PeersIter<impl Iterator<Item = Arc<Peer>>> {
		let peers = match self.peers.try_read_for(LOCK_TIMEOUT) {
			Some(peers) => peers.values().cloned().collect(),
			None => {
				error!("connected_peers: failed to get peers lock");
				vec![]
			}
		};
		PeersIter {
			iter: peers.into_iter(),
		}
	}

	/// Get a peer we're connected to by address.
	pub fn get_connected_peer(&self, addr: PeerAddr) -> Option<Arc<Peer>> {
		self.iter().connected().by_addr(addr)
	}

	pub fn is_banned(&self, peer_addr: PeerAddr) -> bool {
		if let Ok(peer) = self.store.get_peer(peer_addr) {
			return peer.flags == State::Banned;
		}
		false
	}
	/// Ban a peer, disconnecting it if we're currently connected
	pub fn ban_peer(&self, peer_addr: PeerAddr, ban_reason: ReasonForBan) -> Result<(), Error> {
		// Update the peer in peers db
		self.update_state(peer_addr, State::Banned)?;

		// Update the peer in the peers Vec
		match self.get_connected_peer(peer_addr) {
			Some(peer) => {
				debug!("Banning peer {}", peer_addr);
				// setting peer status will get it removed at the next clean_peer
				peer.send_ban_reason(ban_reason)?;
				peer.set_banned();
				peer.stop();
				let mut peers = self.peers.try_write_for(LOCK_TIMEOUT).ok_or_else(|| {
					error!("ban_peer: failed to get peers lock");
					Error::PeerException
				})?;
				peers.remove(&peer.info.addr);
				Ok(())
			}
			None => Err(Error::PeerNotFound),
		}
	}

	/// Unban a peer, checks if it exists and banned then unban
	pub fn unban_peer(&self, peer_addr: PeerAddr) -> Result<(), Error> {
		debug!("unban_peer: peer {}", peer_addr);
		// check if peer exist
		self.get_peer(peer_addr)?;
		if self.is_banned(peer_addr) {
			self.update_state(peer_addr, State::Healthy)
		} else {
			Err(Error::PeerNotBanned)
		}
	}

	fn broadcast<F>(&self, obj_name: &str, inner: F) -> u32
	where
		F: Fn(&Peer) -> Result<bool, Error>,
	{
		let mut count = 0;

		for p in self.iter().connected() {
			match inner(&p) {
				Ok(true) => count += 1,
				Ok(false) => (),
				Err(e) => {
					debug!(
						"Error sending {:?} to peer {:?}: {:?}",
						obj_name, &p.info.addr, e
					);

					let mut peers = match self.peers.try_write_for(LOCK_TIMEOUT) {
						Some(peers) => peers,
						None => {
							error!("broadcast: failed to get peers lock");
							break;
						}
					};
					p.stop();
					peers.remove(&p.info.addr);
				}
			}
		}
		count
	}

	/// Broadcast a compact block to all our connected peers.
	/// This is only used when initially broadcasting a newly mined block.
	pub fn broadcast_compact_block(&self, b: &core::CompactBlock) {
		let count = self.broadcast("compact block", |p| p.send_compact_block(b));
		debug!(
			"broadcast_compact_block: {}, {} at {}, to {} peers, done.",
			b.hash(),
			b.header.pow.total_difficulty,
			b.header.height,
			count,
		);
	}

	/// Broadcast a block header to all our connected peers.
	/// A peer implementation may drop the broadcast request
	/// if it knows the remote peer already has the header.
	pub fn broadcast_header(&self, bh: &core::BlockHeader) {
		let count = self.broadcast("header", |p| p.send_header(bh));
		debug!(
			"broadcast_header: {}, {} at {}, to {} peers, done.",
			bh.hash(),
			bh.pow.total_difficulty,
			bh.height,
			count,
		);
	}

	/// Broadcasts the provided transaction to all our connected peers.
	/// A peer implementation may drop the broadcast request
	/// if it knows the remote peer already has the transaction.
	pub fn broadcast_transaction(&self, tx: &core::Transaction) {
		let count = self.broadcast("transaction", |p| p.send_transaction(tx));
		debug!(
			"broadcast_transaction: {} to {} peers, done.",
			tx.hash(),
			count,
		);
	}

	/// Ping all our connected peers. Always automatically expects a pong back
	/// or disconnects. This acts as a liveness test.
	pub fn check_all(&self, total_difficulty: Difficulty, height: u64) {
		for p in self.iter().connected() {
			if let Err(e) = p.send_ping(total_difficulty, height) {
				debug!("Error pinging peer {:?}: {:?}", &p.info.addr, e);
				let mut peers = match self.peers.try_write_for(LOCK_TIMEOUT) {
					Some(peers) => peers,
					None => {
						error!("check_all: failed to get peers lock");
						break;
					}
				};
				p.stop();
				peers.remove(&p.info.addr);
			}
		}
	}

	/// Iterator over all peers we know about (stored in our db).
	pub fn peer_data_iter(&self) -> Result<impl Iterator<Item = PeerData>, Error> {
		self.store.peers_iter().map_err(From::from)
	}

	/// Convenience for reading all peer data from the db.
	pub fn all_peer_data(&self) -> Vec<PeerData> {
		self.peer_data_iter()
			.map(|peers| peers.collect())
			.unwrap_or(vec![])
	}

	/// Find peers in store (not necessarily connected) and return their data
	pub fn find_peers(&self, state: State, cap: Capabilities, count: usize) -> Vec<PeerData> {
		match self.store.find_peers(state, cap, count) {
			Ok(peers) => peers,
			Err(e) => {
				error!("failed to find peers: {:?}", e);
				vec![]
			}
		}
	}

	/// Get peer in store by address
	pub fn get_peer(&self, peer_addr: PeerAddr) -> Result<PeerData, Error> {
		self.store.get_peer(peer_addr).map_err(From::from)
	}

	/// Whether we've already seen a peer with the provided address
	pub fn exists_peer(&self, peer_addr: PeerAddr) -> Result<bool, Error> {
		self.store.exists_peer(peer_addr).map_err(From::from)
	}

	/// Saves updated information about a peer
	pub fn save_peer(&self, p: &PeerData) -> Result<(), Error> {
		self.store.save_peer(p).map_err(From::from)
	}

	/// Saves updated information about mulitple peers in batch
	pub fn save_peers(&self, p: Vec<PeerData>) -> Result<(), Error> {
		self.store.save_peers(p).map_err(From::from)
	}

	/// Updates the state of a peer in store
	pub fn update_state(&self, peer_addr: PeerAddr, new_state: State) -> Result<(), Error> {
		self.store
			.update_state(peer_addr, new_state)
			.map_err(From::from)
	}

	/// Iterate over the peer list and prune all peers we have
	/// lost connection to or have been deemed problematic.
	/// Also avoid connected peer count getting too high.
	pub fn clean_peers(
		&self,
		max_inbound_count: usize,
		max_outbound_count: usize,
		config: P2PConfig,
	) {
		let preferred_peers = config.peers_preferred.unwrap_or(PeerAddrs::default());

		let mut rm = vec![];

		// build a list of peers to be cleaned up
		{
			for peer in self.iter() {
				let ref peer: &Peer = peer.as_ref();
				if peer.is_banned() {
					debug!("clean_peers {:?}, peer banned", peer.info.addr);
					rm.push(peer.info.addr.clone());
				} else if !peer.is_connected() {
					debug!("clean_peers {:?}, not connected", peer.info.addr);
					rm.push(peer.info.addr.clone());
				} else if peer.is_abusive() {
					let received = peer.tracker().received_bytes.read().count_per_min();
					let sent = peer.tracker().sent_bytes.read().count_per_min();
					debug!(
						"clean_peers {:?}, abusive ({} sent, {} recv)",
						peer.info.addr, sent, received,
					);
					let _ = self.update_state(peer.info.addr, State::Banned);
					rm.push(peer.info.addr.clone());
				} else {
					let (stuck, diff) = peer.is_stuck();
					match self.adapter.total_difficulty() {
						Ok(total_difficulty) => {
							if stuck && diff < total_difficulty {
								debug!("clean_peers {:?}, stuck peer", peer.info.addr);
								let _ = self.update_state(peer.info.addr, State::Defunct);
								rm.push(peer.info.addr.clone());
							}
						}
						Err(e) => error!("failed to get total difficulty: {:?}", e),
					}
				}
			}
		}

		// closure to build an iterator of our inbound peers
		let outbound_peers = || self.iter().outbound().connected().into_iter();

		// check here to make sure we don't have too many outgoing connections
		// Preferred peers are treated preferentially here.
		// Also choose outbound peers with lowest total difficulty to drop.
		let excess_outgoing_count = outbound_peers().count().saturating_sub(max_outbound_count);
		if excess_outgoing_count > 0 {
			let mut peer_infos: Vec<_> = outbound_peers()
				.map(|x| x.info.clone())
				.filter(|x| !preferred_peers.contains(&x.addr))
				.collect();
			peer_infos.sort_unstable_by_key(|x| x.total_difficulty());
			let mut addrs = peer_infos
				.into_iter()
				.map(|x| x.addr)
				.take(excess_outgoing_count)
				.collect();
			rm.append(&mut addrs);
		}

		// closure to build an iterator of our inbound peers
		let inbound_peers = || self.iter().inbound().connected().into_iter();

		// check here to make sure we don't have too many incoming connections
		let excess_incoming_count = inbound_peers().count().saturating_sub(max_inbound_count);
		if excess_incoming_count > 0 {
			let mut addrs: Vec<_> = inbound_peers()
				.filter(|x| !preferred_peers.contains(&x.info.addr))
				.take(excess_incoming_count)
				.map(|x| x.info.addr)
				.collect();
			rm.append(&mut addrs);
		}

		// now clean up peer map based on the list to remove
		{
			let mut peers = match self.peers.try_write_for(LOCK_TIMEOUT) {
				Some(peers) => peers,
				None => {
					error!("clean_peers: failed to get peers lock");
					return;
				}
			};
			for addr in rm {
				let _ = peers.get(&addr).map(|peer| peer.stop());
				peers.remove(&addr);
			}
		}
	}

	pub fn stop(&self) {
		let mut peers = self.peers.write();
		for peer in peers.values() {
			peer.stop();
		}
		for (_, peer) in peers.drain() {
			peer.wait();
		}
	}

	/// We have enough outbound connected peers
	pub fn enough_outbound_peers(&self) -> bool {
		self.iter().outbound().connected().count()
			>= self.config.peer_min_preferred_outbound_count() as usize
	}

	/// Removes those peers that seem to have expired
	pub fn remove_expired(&self) {
		let now = Utc::now();

		// Delete defunct peers from storage
		let _ = self.store.delete_peers(|peer| {
			let diff = now - Utc.timestamp(peer.last_connected, 0);

			let should_remove = peer.flags == State::Defunct
				&& diff > Duration::seconds(global::PEER_EXPIRATION_REMOVE_TIME);

			if should_remove {
				debug!(
					"removing peer {:?}: last connected {} days {} hours {} minutes ago.",
					peer.addr,
					diff.num_days(),
					diff.num_hours(),
					diff.num_minutes()
				);
			}

			should_remove
		});
	}
}

impl ChainAdapter for Peers {
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
		self.adapter.tx_kernel_received(kernel_hash, peer_info)
	}

	fn transaction_received(
		&self,
		tx: core::Transaction,
		stem: bool,
	) -> Result<bool, chain::Error> {
		self.adapter.transaction_received(tx, stem)
	}

	fn block_received(
		&self,
		b: core::Block,
		peer_info: &PeerInfo,
		opts: chain::Options,
	) -> Result<bool, chain::Error> {
		let hash = b.hash();
		if !self.adapter.block_received(b, peer_info, opts)? {
			// if the peer sent us a block that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			debug!(
				"Received a bad block {} from  {}, the peer will be banned",
				hash, peer_info.addr,
			);
			self.ban_peer(peer_info.addr, ReasonForBan::BadBlock)
				.map_err(|e| chain::Error::Other(format!("ban peer error: {:?}", e)))?;
			Ok(false)
		} else {
			Ok(true)
		}
	}

	fn compact_block_received(
		&self,
		cb: core::CompactBlock,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		let hash = cb.hash();
		if !self.adapter.compact_block_received(cb, peer_info)? {
			// if the peer sent us a block that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			debug!(
				"Received a bad compact block {} from  {}, the peer will be banned",
				hash, peer_info.addr
			);
			self.ban_peer(peer_info.addr, ReasonForBan::BadCompactBlock)
				.map_err(|e| chain::Error::Other(format!("ban peer error: {:?}", e)))?;
			Ok(false)
		} else {
			Ok(true)
		}
	}

	fn header_received(
		&self,
		bh: core::BlockHeader,
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		if !self.adapter.header_received(bh, peer_info)? {
			// if the peer sent us a block header that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			self.ban_peer(peer_info.addr, ReasonForBan::BadBlockHeader)
				.map_err(|e| chain::Error::Other(format!("ban peer error: {:?}", e)))?;
			Ok(false)
		} else {
			Ok(true)
		}
	}

	fn headers_received(
		&self,
		headers: &[core::BlockHeader],
		peer_info: &PeerInfo,
	) -> Result<bool, chain::Error> {
		if !self.adapter.headers_received(headers, peer_info)? {
			// if the peer sent us a block header that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			self.ban_peer(peer_info.addr, ReasonForBan::BadBlockHeader)
				.map_err(|e| chain::Error::Other(format!("ban peer error: {:?}", e)))?;
			Ok(false)
		} else {
			Ok(true)
		}
	}

	fn locate_headers(&self, hs: &[Hash]) -> Result<Vec<core::BlockHeader>, chain::Error> {
		self.adapter.locate_headers(hs)
	}

	fn get_block(&self, h: Hash, peer_info: &PeerInfo) -> Option<core::Block> {
		self.adapter.get_block(h, peer_info)
	}

	fn txhashset_read(&self, h: Hash) -> Option<TxHashSetRead> {
		self.adapter.txhashset_read(h)
	}

	fn txhashset_archive_header(&self) -> Result<core::BlockHeader, chain::Error> {
		self.adapter.txhashset_archive_header()
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
		if self.adapter.txhashset_write(h, txhashset_data, peer_info)? {
			debug!(
				"Received a bad txhashset data from {}, the peer will be banned",
				peer_info.addr
			);
			self.ban_peer(peer_info.addr, ReasonForBan::BadTxHashSet)
				.map_err(|e| chain::Error::Other(format!("ban peer error: {:?}", e)))?;
			Ok(true)
		} else {
			Ok(false)
		}
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

	fn get_kernel_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<Segment<TxKernel>, chain::Error> {
		self.adapter.get_kernel_segment(hash, id)
	}

	fn get_bitmap_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<(Segment<BitmapChunk>, Hash), chain::Error> {
		self.adapter.get_bitmap_segment(hash, id)
	}

	fn get_output_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<(Segment<OutputIdentifier>, Hash), chain::Error> {
		self.adapter.get_output_segment(hash, id)
	}

	fn get_rangeproof_segment(
		&self,
		hash: Hash,
		id: SegmentIdentifier,
	) -> Result<Segment<RangeProof>, chain::Error> {
		self.adapter.get_rangeproof_segment(hash, id)
	}

	fn receive_bitmap_segment(
		&self,
		block_hash: Hash,
		output_root: Hash,
		segment: Segment<BitmapChunk>,
	) -> Result<bool, chain::Error> {
		self.adapter
			.receive_bitmap_segment(block_hash, output_root, segment)
	}

	fn receive_output_segment(
		&self,
		block_hash: Hash,
		bitmap_root: Hash,
		segment: Segment<OutputIdentifier>,
	) -> Result<bool, chain::Error> {
		self.adapter
			.receive_output_segment(block_hash, bitmap_root, segment)
	}

	fn receive_rangeproof_segment(
		&self,
		block_hash: Hash,
		segment: Segment<RangeProof>,
	) -> Result<bool, chain::Error> {
		self.adapter.receive_rangeproof_segment(block_hash, segment)
	}

	fn receive_kernel_segment(
		&self,
		block_hash: Hash,
		segment: Segment<TxKernel>,
	) -> Result<bool, chain::Error> {
		self.adapter.receive_kernel_segment(block_hash, segment)
	}
}

impl NetAdapter for Peers {
	/// Find good peers we know with the provided capability and return their
	/// addresses.
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<PeerAddr> {
		let peers = self.find_peers(State::Healthy, capab, MAX_PEER_ADDRS as usize);
		trace!("find_peer_addrs: {} healthy peers picked", peers.len());
		map_vec!(peers, |p| p.addr)
	}

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, peer_addrs: Vec<PeerAddr>) {
		trace!("Received {} peer addrs, saving.", peer_addrs.len());
		let mut to_save: Vec<PeerData> = Vec::new();
		for pa in peer_addrs {
			if let Ok(e) = self.exists_peer(pa) {
				if e {
					continue;
				}
			}
			let peer = PeerData {
				addr: pa,
				capabilities: Capabilities::UNKNOWN,
				user_agent: "".to_string(),
				flags: State::Healthy,
				last_banned: 0,
				ban_reason: ReasonForBan::None,
				last_connected: Utc::now().timestamp(),
			};
			to_save.push(peer);
		}
		if let Err(e) = self.save_peers(to_save) {
			error!("Could not save received peer addresses: {:?}", e);
		}
	}

	fn peer_difficulty(&self, addr: PeerAddr, diff: Difficulty, height: u64) {
		if let Some(peer) = self.get_connected_peer(addr) {
			peer.info.update(height, diff);
		}
	}

	fn is_banned(&self, addr: PeerAddr) -> bool {
		if let Ok(peer) = self.get_peer(addr) {
			peer.flags == State::Banned
		} else {
			false
		}
	}
}

pub struct PeersIter<I> {
	iter: I,
}

impl<I: Iterator> IntoIterator for PeersIter<I> {
	type Item = I::Item;
	type IntoIter = I;

	fn into_iter(self) -> Self::IntoIter {
		self.iter.into_iter()
	}
}

impl<I: Iterator<Item = Arc<Peer>>> PeersIter<I> {
	/// Filter peers that are currently connected.
	/// Note: This adaptor takes a read lock internally.
	/// So if we are chaining adaptors then defer this toward the end of the chain.
	pub fn connected(self) -> PeersIter<impl Iterator<Item = Arc<Peer>>> {
		PeersIter {
			iter: self.iter.filter(|p| p.is_connected()),
		}
	}

	/// Filter inbound peers.
	pub fn inbound(self) -> PeersIter<impl Iterator<Item = Arc<Peer>>> {
		PeersIter {
			iter: self.iter.filter(|p| p.info.is_inbound()),
		}
	}

	/// Filter outbound peers.
	pub fn outbound(self) -> PeersIter<impl Iterator<Item = Arc<Peer>>> {
		PeersIter {
			iter: self.iter.filter(|p| p.info.is_outbound()),
		}
	}

	/// Filter peers with the provided difficulty comparison fn.
	///
	/// with_difficulty(|x| x > diff)
	///
	/// Note: This adaptor takes a read lock internally for each peer.
	/// So if we are chaining adaptors then put this toward later in the chain.
	pub fn with_difficulty<F>(self, f: F) -> PeersIter<impl Iterator<Item = Arc<Peer>>>
	where
		F: Fn(Difficulty) -> bool,
	{
		PeersIter {
			iter: self.iter.filter(move |p| f(p.info.total_difficulty())),
		}
	}

	/// Filter peers that support the provided capabilities.
	pub fn with_capabilities(
		self,
		cap: Capabilities,
	) -> PeersIter<impl Iterator<Item = Arc<Peer>>> {
		PeersIter {
			iter: self.iter.filter(move |p| p.info.capabilities.contains(cap)),
		}
	}

	pub fn by_addr(&mut self, addr: PeerAddr) -> Option<Arc<Peer>> {
		self.iter.find(|p| p.info.addr == addr)
	}

	/// Choose a random peer from the current (filtered) peers.
	pub fn choose_random(self) -> Option<Arc<Peer>> {
		let mut rng = rand::thread_rng();
		self.iter.choose(&mut rng)
	}

	/// Find the max difficulty of the current (filtered) peers.
	pub fn max_difficulty(self) -> Option<Difficulty> {
		self.iter.map(|p| p.info.total_difficulty()).max()
	}

	/// Count the current (filtered) peers.
	pub fn count(self) -> usize {
		self.iter.count()
	}
}
