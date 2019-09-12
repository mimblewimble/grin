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

use crate::util::RwLock;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use rand::seq::SliceRandom;
use rand::thread_rng;

use crate::chain;
use crate::core::core;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::peer::Peer;
use crate::store::{PeerData, PeerStore, State};
use crate::types::{
	Capabilities, ChainAdapter, Error, NetAdapter, P2PConfig, PeerAddr, PeerInfo, ReasonForBan,
	TxHashSetRead, MAX_PEER_ADDRS,
};
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
		let mut peers = match self.peers.try_write_for(LOCK_TIMEOUT) {
			Some(peers) => peers,
			None => {
				error!("add_connected: failed to get peers lock");
				return Err(Error::Timeout);
			}
		};
		let peer_data = PeerData {
			addr: peer.info.addr,
			capabilities: peer.info.capabilities,
			user_agent: peer.info.user_agent.clone(),
			flags: State::Healthy,
			last_banned: 0,
			ban_reason: ReasonForBan::None,
			last_connected: Utc::now().timestamp(),
		};
		debug!("Saving newly connected peer {}.", peer_data.addr);
		self.save_peer(&peer_data)?;
		peers.insert(peer_data.addr, peer.clone());

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
		let peers = match self.peers.try_read_for(LOCK_TIMEOUT) {
			Some(peers) => peers,
			None => {
				error!("is_known: failed to get peers lock");
				return Err(Error::Internal);
			}
		};
		Ok(peers.contains_key(&addr))
	}

	/// Get vec of peers we are currently connected to.
	pub fn connected_peers(&self) -> Vec<Arc<Peer>> {
		let peers = match self.peers.try_read_for(LOCK_TIMEOUT) {
			Some(peers) => peers,
			None => {
				error!("connected_peers: failed to get peers lock");
				return vec![];
			}
		};
		let mut res = peers
			.values()
			.filter(|p| p.is_connected())
			.cloned()
			.collect::<Vec<_>>();
		res.shuffle(&mut thread_rng());
		res
	}

	/// Get vec of peers we currently have an outgoing connection with.
	pub fn outgoing_connected_peers(&self) -> Vec<Arc<Peer>> {
		self.connected_peers()
			.into_iter()
			.filter(|x| x.info.is_outbound())
			.collect()
	}

	/// Get vec of peers we currently have an incoming connection with.
	pub fn incoming_connected_peers(&self) -> Vec<Arc<Peer>> {
		self.connected_peers()
			.into_iter()
			.filter(|x| x.info.is_inbound())
			.collect()
	}

	/// Get a peer we're connected to by address.
	pub fn get_connected_peer(&self, addr: PeerAddr) -> Option<Arc<Peer>> {
		let peers = match self.peers.try_read_for(LOCK_TIMEOUT) {
			Some(peers) => peers,
			None => {
				error!("get_connected_peer: failed to get peers lock");
				return None;
			}
		};
		peers.get(&addr).map(|p| p.clone())
	}

	/// Number of peers currently connected to.
	pub fn peer_count(&self) -> u32 {
		self.connected_peers().len() as u32
	}

	/// Number of outbound peers currently connected to.
	pub fn peer_outbound_count(&self) -> u32 {
		self.outgoing_connected_peers().len() as u32
	}

	/// Number of inbound peers currently connected to.
	pub fn peer_inbound_count(&self) -> u32 {
		self.incoming_connected_peers().len() as u32
	}

	// Return vec of connected peers that currently advertise more work
	// (total_difficulty) than we do.
	pub fn more_work_peers(&self) -> Result<Vec<Arc<Peer>>, chain::Error> {
		let peers = self.connected_peers();
		if peers.len() == 0 {
			return Ok(vec![]);
		}

		let total_difficulty = self.total_difficulty()?;

		let mut max_peers = peers
			.into_iter()
			.filter(|x| x.info.total_difficulty() > total_difficulty)
			.collect::<Vec<_>>();

		max_peers.shuffle(&mut thread_rng());
		Ok(max_peers)
	}

	// Return number of connected peers that currently advertise more/same work
	// (total_difficulty) than/as we do.
	pub fn more_or_same_work_peers(&self) -> Result<usize, chain::Error> {
		let peers = self.connected_peers();
		if peers.len() == 0 {
			return Ok(0);
		}

		let total_difficulty = self.total_difficulty()?;

		Ok(peers
			.iter()
			.filter(|x| x.info.total_difficulty() >= total_difficulty)
			.count())
	}

	/// Returns single random peer with more work than us.
	pub fn more_work_peer(&self) -> Option<Arc<Peer>> {
		match self.more_work_peers() {
			Ok(mut peers) => peers.pop(),
			Err(e) => {
				error!("failed to get more work peers: {:?}", e);
				None
			}
		}
	}

	/// Return vec of connected peers that currently have the most worked
	/// branch, showing the highest total difficulty.
	pub fn most_work_peers(&self) -> Vec<Arc<Peer>> {
		let peers = self.connected_peers();
		if peers.len() == 0 {
			return vec![];
		}

		let max_total_difficulty = match peers.iter().map(|x| x.info.total_difficulty()).max() {
			Some(v) => v,
			None => return vec![],
		};

		let mut max_peers = peers
			.into_iter()
			.filter(|x| x.info.total_difficulty() == max_total_difficulty)
			.collect::<Vec<_>>();

		max_peers.shuffle(&mut thread_rng());
		max_peers
	}

	/// Returns single random peer with the most worked branch, showing the
	/// highest total difficulty.
	pub fn most_work_peer(&self) -> Option<Arc<Peer>> {
		self.most_work_peers().pop()
	}

	pub fn is_banned(&self, peer_addr: PeerAddr) -> bool {
		if let Ok(peer) = self.store.get_peer(peer_addr) {
			return peer.flags == State::Banned;
		}
		false
	}

	/// Ban a peer, disconnecting it if we're currently connected
	pub fn ban_peer(&self, peer_addr: PeerAddr, ban_reason: ReasonForBan) {
		if let Err(e) = self.update_state(peer_addr, State::Banned) {
			error!("Couldn't ban {}: {:?}", peer_addr, e);
			return;
		}

		if let Some(peer) = self.get_connected_peer(peer_addr) {
			debug!("Banning peer {}", peer_addr);
			// setting peer status will get it removed at the next clean_peer
			match peer.send_ban_reason(ban_reason) {
				Err(e) => error!("failed to send a ban reason to{}: {:?}", peer_addr, e),
				Ok(_) => debug!("ban reason {:?} was sent to {}", ban_reason, peer_addr),
			};
			peer.set_banned();
			peer.stop();

			let mut peers = match self.peers.try_write_for(LOCK_TIMEOUT) {
				Some(peers) => peers,
				None => {
					error!("ban_peer: failed to get peers lock");
					return;
				}
			};
			peers.remove(&peer.info.addr);
		}
	}

	/// Unban a peer, checks if it exists and banned then unban
	pub fn unban_peer(&self, peer_addr: PeerAddr) {
		debug!("unban_peer: peer {}", peer_addr);
		match self.get_peer(peer_addr) {
			Ok(_) => {
				if self.is_banned(peer_addr) {
					if let Err(e) = self.update_state(peer_addr, State::Healthy) {
						error!("Couldn't unban {}: {:?}", peer_addr, e);
					}
				} else {
					error!("Couldn't unban {}: peer is not banned", peer_addr);
				}
			}
			Err(e) => error!("Couldn't unban {}: {:?}", peer_addr, e),
		};
	}

	fn broadcast<F>(&self, obj_name: &str, inner: F) -> u32
	where
		F: Fn(&Peer) -> Result<bool, Error>,
	{
		let mut count = 0;

		for p in self.connected_peers().iter() {
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
		for p in self.connected_peers().iter() {
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

	/// All peer information we have in storage
	pub fn all_peers(&self) -> Vec<PeerData> {
		match self.store.all_peers() {
			Ok(peers) => peers,
			Err(e) => {
				error!("all_peers failed: {:?}", e);
				vec![]
			}
		}
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

	/// Updates the state of a peer in store
	pub fn update_state(&self, peer_addr: PeerAddr, new_state: State) -> Result<(), Error> {
		self.store
			.update_state(peer_addr, new_state)
			.map_err(From::from)
	}

	/// Iterate over the peer list and prune all peers we have
	/// lost connection to or have been deemed problematic.
	/// Also avoid connected peer count getting too high.
	pub fn clean_peers(&self, max_inbound_count: usize, max_outbound_count: usize) {
		let mut rm = vec![];

		// build a list of peers to be cleaned up
		{
			let peers = match self.peers.try_read_for(LOCK_TIMEOUT) {
				Some(peers) => peers,
				None => {
					error!("clean_peers: can't get peers lock");
					return;
				}
			};
			for peer in peers.values() {
				if peer.is_banned() {
					debug!("clean_peers {:?}, peer banned", peer.info.addr);
					rm.push(peer.info.addr.clone());
				} else if !peer.is_connected() {
					debug!("clean_peers {:?}, not connected", peer.info.addr);
					rm.push(peer.info.addr.clone());
				} else if peer.is_abusive() {
					if let Some(counts) = peer.last_min_message_counts() {
						debug!(
							"clean_peers {:?}, abusive ({} sent, {} recv)",
							peer.info.addr, counts.0, counts.1,
						);
					}
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

		// check here to make sure we don't have too many outgoing connections
		let excess_outgoing_count =
			(self.peer_outbound_count() as usize).saturating_sub(max_outbound_count);
		if excess_outgoing_count > 0 {
			let mut addrs = self
				.outgoing_connected_peers()
				.iter()
				.take(excess_outgoing_count)
				.map(|x| x.info.addr.clone())
				.collect::<Vec<_>>();
			rm.append(&mut addrs);
		}

		// check here to make sure we don't have too many incoming connections
		let excess_incoming_count =
			(self.peer_inbound_count() as usize).saturating_sub(max_inbound_count);
		if excess_incoming_count > 0 {
			let mut addrs = self
				.incoming_connected_peers()
				.iter()
				.take(excess_incoming_count)
				.map(|x| x.info.addr.clone())
				.collect::<Vec<_>>();
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
		self.peer_outbound_count() >= self.config.peer_min_preferred_outbound_count()
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
		was_requested: bool,
	) -> Result<bool, chain::Error> {
		let hash = b.hash();
		if !self.adapter.block_received(b, peer_info, was_requested)? {
			// if the peer sent us a block that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			debug!(
				"Received a bad block {} from  {}, the peer will be banned",
				hash, peer_info.addr,
			);
			self.ban_peer(peer_info.addr, ReasonForBan::BadBlock);
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
			self.ban_peer(peer_info.addr, ReasonForBan::BadCompactBlock);
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
			self.ban_peer(peer_info.addr, ReasonForBan::BadBlockHeader);
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
			self.ban_peer(peer_info.addr, ReasonForBan::BadBlockHeader);
			Ok(false)
		} else {
			Ok(true)
		}
	}

	fn locate_headers(&self, hs: &[Hash]) -> Result<Vec<core::BlockHeader>, chain::Error> {
		self.adapter.locate_headers(hs)
	}

	fn get_block(&self, h: Hash) -> Option<core::Block> {
		self.adapter.get_block(h)
	}

	fn kernel_data_read(&self) -> Result<File, chain::Error> {
		self.adapter.kernel_data_read()
	}

	fn kernel_data_write(&self, reader: &mut dyn Read) -> Result<bool, chain::Error> {
		self.adapter.kernel_data_write(reader)
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
			self.ban_peer(peer_info.addr, ReasonForBan::BadTxHashSet);
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
			if let Err(e) = self.save_peer(&peer) {
				error!("Could not save received peer address: {:?}", e);
			}
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
