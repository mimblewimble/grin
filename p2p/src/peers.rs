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
use std::net::SocketAddr;
use std::sync::Arc;

use rand::{thread_rng, Rng};

use crate::core::core;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::global;
use crate::core::pow::Difficulty;
use chrono::prelude::*;
use chrono::Duration;

use crate::peer::Peer;
use crate::store::{PeerData, PeerStore, State};
use crate::types::{
	Capabilities, ChainAdapter, Direction, Error, NetAdapter, P2PConfig, ReasonForBan,
	TxHashSetRead, MAX_PEER_ADDRS,
};

pub struct Peers {
	pub adapter: Arc<dyn ChainAdapter>,
	store: PeerStore,
	peers: RwLock<HashMap<SocketAddr, Arc<Peer>>>,
	dandelion_relay: RwLock<HashMap<i64, Arc<Peer>>>,
	config: P2PConfig,
}

impl Peers {
	pub fn new(store: PeerStore, adapter: Arc<dyn ChainAdapter>, config: P2PConfig) -> Peers {
		Peers {
			adapter,
			store,
			config,
			peers: RwLock::new(HashMap::new()),
			dandelion_relay: RwLock::new(HashMap::new()),
		}
	}

	/// Adds the peer to our internal peer mapping. Note that the peer is still
	/// returned so the server can run it.
	pub fn add_connected(&self, peer: Arc<Peer>) -> Result<(), Error> {
		let peer_data: PeerData;
		let addr: SocketAddr;
		{
			peer_data = PeerData {
				addr: peer.info.addr,
				capabilities: peer.info.capabilities,
				user_agent: peer.info.user_agent.clone(),
				flags: State::Healthy,
				last_banned: 0,
				ban_reason: ReasonForBan::None,
				last_connected: Utc::now().timestamp(),
			};
			addr = peer.info.addr.clone();
		}
		debug!("Saving newly connected peer {}.", addr);
		self.save_peer(&peer_data)?;

		{
			let mut peers = self.peers.write();
			peers.insert(addr, peer.clone());
		}
		Ok(())
	}

	// Update the dandelion relay
	pub fn update_dandelion_relay(&self) {
		let peers = self.outgoing_connected_peers();

		let peer = &self
			.config
			.dandelion_peer
			.and_then(|ip| peers.iter().find(|x| x.info.addr == ip))
			.or(thread_rng().choose(&peers));

		match peer {
			Some(peer) => self.set_dandelion_relay(peer),
			None => debug!("Could not update dandelion relay"),
		}
	}

	fn set_dandelion_relay(&self, peer: &Arc<Peer>) {
		// Clear the map and add new relay
		let dandelion_relay = &self.dandelion_relay;
		dandelion_relay.write().clear();
		dandelion_relay
			.write()
			.insert(Utc::now().timestamp(), peer.clone());
		debug!(
			"Successfully updated Dandelion relay to: {}",
			peer.info.addr
		);
	}

	// Get the dandelion relay
	pub fn get_dandelion_relay(&self) -> HashMap<i64, Arc<Peer>> {
		self.dandelion_relay.read().clone()
	}

	pub fn is_known(&self, addr: &SocketAddr) -> bool {
		self.peers.read().contains_key(addr)
	}

	/// Check whether an ip address is in the active peers list, ignore the port
	pub fn is_known_ip(&self, addr: &SocketAddr) -> bool {
		for socket in self.peers.read().keys() {
			if addr.ip() == socket.ip() {
				return true;
			}
		}
		return false;
	}

	/// Get vec of peers we are currently connected to.
	pub fn connected_peers(&self) -> Vec<Arc<Peer>> {
		let mut res = self
			.peers
			.read()
			.values()
			.filter(|p| p.is_connected())
			.cloned()
			.collect::<Vec<_>>();
		thread_rng().shuffle(&mut res);
		res
	}

	pub fn outgoing_connected_peers(&self) -> Vec<Arc<Peer>> {
		let peers = self.connected_peers();
		let res = peers
			.into_iter()
			.filter(|x| x.info.direction == Direction::Outbound)
			.collect::<Vec<_>>();
		res
	}

	/// Get a peer we're connected to by address.
	pub fn get_connected_peer(&self, addr: &SocketAddr) -> Option<Arc<Peer>> {
		self.peers.read().get(addr).map(|p| p.clone())
	}

	/// Number of peers we're currently connected to.
	pub fn peer_count(&self) -> u32 {
		self.peers
			.read()
			.values()
			.filter(|x| x.is_connected())
			.count() as u32
	}

	// Return vec of connected peers that currently advertise more work
	// (total_difficulty) than we do.
	pub fn more_work_peers(&self) -> Vec<Arc<Peer>> {
		let peers = self.connected_peers();
		if peers.len() == 0 {
			return vec![];
		}

		let total_difficulty = self.total_difficulty();

		let mut max_peers = peers
			.into_iter()
			.filter(|x| x.info.total_difficulty() > total_difficulty)
			.collect::<Vec<_>>();

		thread_rng().shuffle(&mut max_peers);
		max_peers
	}

	/// Returns single random peer with more work than us.
	pub fn more_work_peer(&self) -> Option<Arc<Peer>> {
		self.more_work_peers().pop()
	}

	/// Return vec of connected peers that currently have the most worked
	/// branch, showing the highest total difficulty.
	pub fn most_work_peers(&self) -> Vec<Arc<Peer>> {
		let peers = self.connected_peers();
		if peers.len() == 0 {
			return vec![];
		}

		let max_total_difficulty = peers
			.iter()
			.map(|x| x.info.total_difficulty())
			.max()
			.unwrap();

		let mut max_peers = peers
			.into_iter()
			.filter(|x| x.info.total_difficulty() == max_total_difficulty)
			.collect::<Vec<_>>();

		thread_rng().shuffle(&mut max_peers);
		max_peers
	}

	/// Returns single random peer with the most worked branch, showing the
	/// highest total difficulty.
	pub fn most_work_peer(&self) -> Option<Arc<Peer>> {
		self.most_work_peers().pop()
	}

	pub fn is_banned(&self, peer_addr: SocketAddr) -> bool {
		if global::is_production_mode() {
			// Ban only cares about ip address, no mather what port.
			// so, we query all saved peers with one same ip address, and ignore port
			let peers_data = self.store.find_peers_by_ip(peer_addr);
			for peer_data in peers_data {
				if peer_data.flags == State::Banned {
					return true;
				}
			}
		} else {
			// For travis-ci test, we need run multiple nodes in one server, with same ip address.
			// so, just query the ip address and the port
			if let Ok(peer_data) = self.store.get_peer(peer_addr) {
				if peer_data.flags == State::Banned {
					return true;
				}
			}
		}
		false
	}

	/// Ban a peer, disconnecting it if we're currently connected
	pub fn ban_peer(&self, peer_addr: &SocketAddr, ban_reason: ReasonForBan) {
		if let Err(e) = self.update_state(*peer_addr, State::Banned) {
			error!("Couldn't ban {}: {:?}", peer_addr, e);
		}

		if let Some(peer) = self.get_connected_peer(peer_addr) {
			debug!("Banning peer {}", peer_addr);
			// setting peer status will get it removed at the next clean_peer
			peer.send_ban_reason(ban_reason);
			peer.set_banned();
			peer.stop();
		}
	}

	/// Unban a peer, checks if it exists and banned then unban
	pub fn unban_peer(&self, peer_addr: &SocketAddr) {
		debug!("unban_peer: peer {}", peer_addr);
		match self.get_peer(*peer_addr) {
			Ok(_) => {
				if self.is_banned(*peer_addr) {
					if let Err(e) = self.update_state(*peer_addr, State::Healthy) {
						error!("Couldn't unban {}: {:?}", peer_addr, e);
					}
				} else {
					error!("Couldn't unban {}: peer is not banned", peer_addr);
				}
			}
			Err(e) => error!("Couldn't unban {}: {:?}", peer_addr, e),
		};
	}

	fn broadcast<F>(&self, obj_name: &str, num_peers: u32, inner: F) -> u32
	where
		F: Fn(&Peer) -> Result<bool, Error>,
	{
		let mut count = 0;

		// Iterate over our connected peers.
		// Try our best to send to at most num_peers peers.
		for p in self.connected_peers().iter() {
			match inner(&p) {
				Ok(true) => count += 1,
				Ok(false) => (),
				Err(e) => debug!("Error sending {} to peer: {:?}", obj_name, e),
			}

			if count >= num_peers {
				break;
			}
		}
		count
	}

	/// Broadcasts the provided compact block to PEER_MAX_COUNT of our peers.
	/// This is only used when initially broadcasting a newly mined block
	/// from a mining node so we want to broadcast it far and wide.
	/// A peer implementation may drop the broadcast request
	/// if it knows the remote peer already has the block.
	pub fn broadcast_compact_block(&self, b: &core::CompactBlock) {
		let num_peers = self.config.peer_max_count();
		let count = self.broadcast("compact block", num_peers, |p| p.send_compact_block(b));
		debug!(
			"broadcast_compact_block: {}, {} at {}, to {} peers, done.",
			b.hash(),
			b.header.pow.total_difficulty,
			b.header.height,
			count,
		);
	}

	/// Broadcasts the provided header to PEER_PREFERRED_COUNT of our peers.
	/// We may be connected to PEER_MAX_COUNT peers so we only
	/// want to broadcast to a random subset of peers.
	/// A peer implementation may drop the broadcast request
	/// if it knows the remote peer already has the header.
	pub fn broadcast_header(&self, bh: &core::BlockHeader) {
		let num_peers = self.config.peer_min_preferred_count();
		let count = self.broadcast("header", num_peers, |p| p.send_header(bh));
		debug!(
			"broadcast_header: {}, {} at {}, to {} peers, done.",
			bh.hash(),
			bh.pow.total_difficulty,
			bh.height,
			count,
		);
	}

	/// Relays the provided stem transaction to our single stem peer.
	pub fn relay_stem_transaction(&self, tx: &core::Transaction) -> Result<(), Error> {
		let dandelion_relay = self.get_dandelion_relay();
		if dandelion_relay.is_empty() {
			debug!("No dandelion relay, updating.");
			self.update_dandelion_relay();
		}
		// If still return an error, let the caller handle this as they see fit.
		// The caller will "fluff" at this point as the stem phase is finished.
		if dandelion_relay.is_empty() {
			return Err(Error::NoDandelionRelay);
		}
		for relay in dandelion_relay.values() {
			if relay.is_connected() {
				if let Err(e) = relay.send_stem_transaction(tx) {
					debug!("Error sending stem transaction to peer relay: {:?}", e);
				}
			}
		}
		Ok(())
	}

	/// Broadcasts the provided transaction to PEER_PREFERRED_COUNT of our
	/// peers. We may be connected to PEER_MAX_COUNT peers so we only
	/// want to broadcast to a random subset of peers.
	/// A peer implementation may drop the broadcast request
	/// if it knows the remote peer already has the transaction.
	pub fn broadcast_transaction(&self, tx: &core::Transaction) {
		let num_peers = self.config.peer_max_count();
		let count = self.broadcast("transaction", num_peers, |p| p.send_transaction(tx));
		debug!(
			"broadcast_transaction: {} to {} peers, done.",
			tx.hash(),
			count,
		);
	}

	/// Ping all our connected peers. Always automatically expects a pong back
	/// or disconnects. This acts as a liveness test.
	pub fn check_all(&self, total_difficulty: Difficulty, height: u64) {
		let peers_map = self.peers.read();
		for p in peers_map.values() {
			if p.is_connected() {
				let _ = p.send_ping(total_difficulty, height);
			}
		}
	}

	/// All peer information we have in storage
	pub fn all_peers(&self) -> Vec<PeerData> {
		self.store.all_peers()
	}

	/// Find peers in store (not necessarily connected) and return their data
	pub fn find_peers(&self, state: State, cap: Capabilities, count: usize) -> Vec<PeerData> {
		self.store.find_peers(state, cap, count)
	}

	/// Get peer in store by address
	pub fn get_peer(&self, peer_addr: SocketAddr) -> Result<PeerData, Error> {
		self.store.get_peer(peer_addr).map_err(From::from)
	}

	/// Whether we've already seen a peer with the provided address
	pub fn exists_peer(&self, peer_addr: SocketAddr) -> Result<bool, Error> {
		self.store.exists_peer(peer_addr).map_err(From::from)
	}

	/// Saves updated information about a peer
	pub fn save_peer(&self, p: &PeerData) -> Result<(), Error> {
		self.store.save_peer(p).map_err(From::from)
	}

	/// Updates the state of a peer in store
	pub fn update_state(&self, peer_addr: SocketAddr, new_state: State) -> Result<(), Error> {
		self.store
			.update_state(peer_addr, new_state)
			.map_err(From::from)
	}

	/// Iterate over the peer list and prune all peers we have
	/// lost connection to or have been deemed problematic.
	/// Also avoid connected peer count getting too high.
	pub fn clean_peers(&self, max_count: usize) {
		let mut rm = vec![];

		// build a list of peers to be cleaned up
		for peer in self.peers.read().values() {
			if peer.is_banned() {
				debug!("clean_peers {:?}, peer banned", peer.info.addr);
				rm.push(peer.info.addr.clone());
			} else if !peer.is_connected() {
				debug!("clean_peers {:?}, not connected", peer.info.addr);
				rm.push(peer.info.addr.clone());
			} else if peer.is_abusive() {
				let counts = peer.last_min_message_counts().unwrap();
				debug!(
					"clean_peers {:?}, abusive ({} sent, {} recv)",
					peer.info.addr, counts.0, counts.1,
				);
				let _ = self.update_state(peer.info.addr, State::Banned);
				rm.push(peer.info.addr.clone());
			} else {
				let (stuck, diff) = peer.is_stuck();
				if stuck && diff < self.adapter.total_difficulty() {
					debug!("clean_peers {:?}, stuck peer", peer.info.addr);
					let _ = self.update_state(peer.info.addr, State::Defunct);
					rm.push(peer.info.addr.clone());
				}
			}
		}

		// ensure we do not still have too many connected peers
		let excess_count = (self.peer_count() as usize)
			.saturating_sub(rm.len())
			.saturating_sub(max_count);
		if excess_count > 0 {
			// map peers to addrs in a block to bound how long we keep the read lock for
			let mut addrs = self
				.connected_peers()
				.iter()
				.take(excess_count)
				.map(|x| x.info.addr.clone())
				.collect::<Vec<_>>();
			rm.append(&mut addrs);
		}

		// now clean up peer map based on the list to remove
		{
			let mut peers = self.peers.write();
			for p in rm {
				let _ = peers.get(&p).map(|p| p.stop());
				peers.remove(&p);
			}
		}
	}

	pub fn stop(&self) {
		let mut peers = self.peers.write();
		for (_, peer) in peers.drain() {
			peer.stop();
		}
	}

	pub fn enough_peers(&self) -> bool {
		self.connected_peers().len() >= self.config.peer_min_preferred_count() as usize
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
	fn total_difficulty(&self) -> Difficulty {
		self.adapter.total_difficulty()
	}

	fn total_height(&self) -> u64 {
		self.adapter.total_height()
	}

	fn get_transaction(&self, kernel_hash: Hash) -> Option<core::Transaction> {
		self.adapter.get_transaction(kernel_hash)
	}

	fn tx_kernel_received(&self, kernel_hash: Hash, addr: SocketAddr) {
		self.adapter.tx_kernel_received(kernel_hash, addr)
	}

	fn transaction_received(&self, tx: core::Transaction, stem: bool) {
		self.adapter.transaction_received(tx, stem)
	}

	fn block_received(&self, b: core::Block, peer_addr: SocketAddr) -> bool {
		let hash = b.hash();
		if !self.adapter.block_received(b, peer_addr) {
			// if the peer sent us a block that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			debug!(
				"Received a bad block {} from  {}, the peer will be banned",
				hash, peer_addr
			);
			self.ban_peer(&peer_addr, ReasonForBan::BadBlock);
			false
		} else {
			true
		}
	}

	fn compact_block_received(&self, cb: core::CompactBlock, peer_addr: SocketAddr) -> bool {
		let hash = cb.hash();
		if !self.adapter.compact_block_received(cb, peer_addr) {
			// if the peer sent us a block that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			debug!(
				"Received a bad compact block {} from  {}, the peer will be banned",
				hash, &peer_addr
			);
			self.ban_peer(&peer_addr, ReasonForBan::BadCompactBlock);
			false
		} else {
			true
		}
	}

	fn header_received(&self, bh: core::BlockHeader, peer_addr: SocketAddr) -> bool {
		if !self.adapter.header_received(bh, peer_addr) {
			// if the peer sent us a block header that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			self.ban_peer(&peer_addr, ReasonForBan::BadBlockHeader);
			false
		} else {
			true
		}
	}

	fn headers_received(&self, headers: &[core::BlockHeader], peer_addr: SocketAddr) -> bool {
		if !self.adapter.headers_received(headers, peer_addr) {
			// if the peer sent us a block header that's intrinsically bad
			// they are either mistaken or malevolent, both of which require a ban
			self.ban_peer(&peer_addr, ReasonForBan::BadBlockHeader);
			false
		} else {
			true
		}
	}

	fn locate_headers(&self, hs: &[Hash]) -> Vec<core::BlockHeader> {
		self.adapter.locate_headers(hs)
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
		if !self.adapter.txhashset_write(h, txhashset_data, peer_addr) {
			debug!(
				"Received a bad txhashset data from {}, the peer will be banned",
				&peer_addr
			);
			self.ban_peer(&peer_addr, ReasonForBan::BadTxHashSet);
			false
		} else {
			true
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
}

impl NetAdapter for Peers {
	/// Find good peers we know with the provided capability and return their
	/// addresses.
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<SocketAddr> {
		let peers = self.find_peers(State::Healthy, capab, MAX_PEER_ADDRS as usize);
		trace!("find_peer_addrs: {} healthy peers picked", peers.len());
		map_vec!(peers, |p| p.addr)
	}

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, peer_addrs: Vec<SocketAddr>) {
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

	fn peer_difficulty(&self, addr: SocketAddr, diff: Difficulty, height: u64) {
		if let Some(peer) = self.get_connected_peer(&addr) {
			peer.info.update(height, diff);
		}
	}

	fn is_banned(&self, addr: SocketAddr) -> bool {
		if let Ok(peer) = self.get_peer(addr) {
			peer.flags == State::Banned
		} else {
			false
		}
	}
}
