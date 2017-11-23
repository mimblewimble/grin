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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::thread;

use chain::{self, ChainAdapter};
use core::core::{self, Output};
use core::core::block::BlockHeader;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use p2p::{self, NetAdapter, Peer, PeerData, PeerStore, Server, State};
use pool;
use util::secp::pedersen::Commitment;
use util::OneTime;
use store;
use sync;
use util::LOGGER;

/// Implementation of the NetAdapter for the blockchain. Gets notified when new
/// blocks and transactions are received and forwards to the chain and pool
/// implementations.
pub struct NetToChainAdapter {
	chain: Arc<chain::Chain>,
	peer_store: Arc<PeerStore>,
	connected_peers: Arc<RwLock<HashMap<SocketAddr, Arc<RwLock<Peer>>>>>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	syncer: OneTime<Arc<sync::Syncer>>,
}

impl NetAdapter for NetToChainAdapter {
	fn total_difficulty(&self) -> Difficulty {
		self.chain.total_difficulty()
	}

	fn transaction_received(&self, tx: core::Transaction) {
		let source = pool::TxSource {
			debug_name: "p2p".to_string(),
			identifier: "?.?.?.?".to_string(),
		};
		debug!(
			LOGGER,
			"Received tx {} from {}, going to process.",
			tx.hash(),
			source.identifier,
		);

		let h = tx.hash();
		if let Err(e) = self.tx_pool.write().unwrap().add_to_memory_pool(source, tx) {
			debug!(LOGGER, "Transaction {} rejected: {:?}", h, e);
		}
	}

	fn block_received(&self, b: core::Block) {
		let bhash = b.hash();
		debug!(
			LOGGER,
			"Received block {} at {} from network, going to process.",
			bhash,
			b.header.height,
		);

		// pushing the new block through the chain pipeline
		let res = self.chain.process_block(b, self.chain_opts());

		if let &Err(ref e) = &res {
			debug!(LOGGER, "Block {} refused by chain: {:?}", bhash, e);
		}

		if self.syncing() {
			// always notify the syncer we received a block
			// otherwise we jam up the 8 download slots with orphans
			debug!(LOGGER, "adapter: notifying syncer: received block {:?}", bhash);
			self.syncer.borrow().block_received(bhash);
		}
	}

	fn headers_received(&self, bhs: Vec<core::BlockHeader>) {
		info!(
			LOGGER,
			"Received {} block headers",
			bhs.len(),
		);

		// try to add each header to our header chain
		let mut added_hs = vec![];
		for bh in bhs {
			let res = self.chain.process_block_header(&bh, self.chain_opts());
			match res {
				Ok(_) => {
					added_hs.push(bh.hash());
				}
				Err(chain::Error::Unfit(s)) => {
					info!(
						LOGGER,
						"Received unfit block header {} at {}: {}.",
						bh.hash(),
						bh.height,
						s
					);
				}
				Err(chain::Error::StoreErr(e, explanation)) => {
					error!(
						LOGGER,
						"Store error processing block header {}: in {} {:?}",
						bh.hash(),
						explanation,
						e
					);
					return;
				}
				Err(e) => {
					info!(LOGGER, "Invalid block header {}: {:?}.", bh.hash(), e);
					// TODO penalize peer somehow
				}
			}
		}
		info!(
			LOGGER,
			"Added {} headers to the header chain.",
			added_hs.len()
		);

		if self.syncing() {
			self.syncer.borrow().headers_received(added_hs);
		}
	}

	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader> {
		debug!(
			LOGGER,
			"locate_headers: {:?}",
			locator,
		);

		if locator.len() == 0 {
			return vec![];
		}

		// recursively go back through the locator vector
		// and stop when we find a header that we recognize
		// this will be a header shared in common between us and the peer
		let known = self.chain.get_block_header(&locator[0]);
		let header = match known {
			Ok(header) => header,
			Err(chain::Error::StoreErr(store::Error::NotFoundErr, _)) => {
				return self.locate_headers(locator[1..].to_vec());
			}
			Err(e) => {
				error!(LOGGER, "Could not build header locator: {:?}", e);
				return vec![];
			}
		};

		debug!(
			LOGGER,
			"locate_headers: common header: {:?}",
			header.hash(),
		);

		// looks like we know one, getting as many following headers as allowed
		let hh = header.height;
		let mut headers = vec![];
		for h in (hh + 1)..(hh + (p2p::MAX_BLOCK_HEADERS as u64)) {
			let header = self.chain.get_header_by_height(h);
			match header {
				Ok(head) => headers.push(head),
				Err(chain::Error::StoreErr(store::Error::NotFoundErr, _)) => break,
				Err(e) => {
					error!(LOGGER, "Could not build header locator: {:?}", e);
					return vec![];
				}
			}
		}

		debug!(
			LOGGER,
			"locate_headers: returning headers: {}",
			headers.len(),
		);

		headers
	}

	/// Gets a full block by its hash.
	fn get_block(&self, h: Hash) -> Option<core::Block> {
		let b = self.chain.get_block(&h);
		match b {
			Ok(b) => Some(b),
			_ => None,
		}
	}

	/// Find good peers we know with the provided capability and return their
	/// addresses.
	fn find_peer_addrs(&self, capab: p2p::Capabilities) -> Vec<SocketAddr> {
		let peers = self.peer_store
			.find_peers(State::Healthy, capab, p2p::MAX_PEER_ADDRS as usize);
		debug!(LOGGER, "Got {} peer addrs to send.", peers.len());
		map_vec!(peers, |p| p.addr)
	}

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, peer_addrs: Vec<SocketAddr>) {
		debug!(LOGGER, "Received {} peer addrs, saving.", peer_addrs.len());
		for pa in peer_addrs {
			if let Ok(e) = self.peer_store.exists_peer(pa) {
				if e {
					continue;
				}
			}
			let peer = PeerData {
				addr: pa,
				capabilities: p2p::UNKNOWN,
				user_agent: "".to_string(),
				flags: State::Healthy,
			};
			if let Err(e) = self.peer_store.save_peer(&peer) {
				error!(LOGGER, "Could not save received peer address: {:?}", e);
			}
		}
	}

	/// Network successfully connected to a peer.
	fn peer_connected(&self, pi: &p2p::PeerInfo) {
		debug!(LOGGER, "Saving newly connected peer {}.", pi.addr);
		let peer = PeerData {
			addr: pi.addr,
			capabilities: pi.capabilities,
			user_agent: pi.user_agent.clone(),
			flags: State::Healthy,
		};
		if let Err(e) = self.peer_store.save_peer(&peer) {
			error!(LOGGER, "Could not save connected peer: {:?}", e);
		}
	}

	fn peer_difficulty(&self, addr: SocketAddr, diff: Difficulty) {
		debug!(
			LOGGER,
			"peer total_diff (ping/pong): {}, {} vs us {}",
			addr,
			diff,
			self.total_difficulty(),
		);

		if diff.into_num() > 0 {
			let peers = self.connected_peers.read().unwrap();
			if let Some(peer) = peers.get(&addr) {
				let mut peer = peer.write().unwrap();
				peer.info.total_difficulty = diff;
			}
		}
	}
}

impl NetToChainAdapter {
	pub fn new(
		chain_ref: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
		peer_store: Arc<PeerStore>,
		connected_peers: Arc<RwLock<HashMap<SocketAddr, Arc<RwLock<Peer>>>>>,
	) -> NetToChainAdapter {
		NetToChainAdapter {
			chain: chain_ref,
			peer_store: peer_store,
			connected_peers: connected_peers,
			tx_pool: tx_pool,
			syncer: OneTime::new(),
		}
	}

	/// Start syncing the chain by instantiating and running the Syncer in the
	/// background (a new thread is created).
	pub fn start_sync(&self, sync: sync::Syncer) {
		let arc_sync = Arc::new(sync);
		self.syncer.init(arc_sync.clone());
		let _ = thread::Builder::new()
			.name("syncer".to_string())
			.spawn(move || {
				let res = arc_sync.run();
				if let Err(e) = res {
					panic!("Error during sync, aborting: {:?}", e);
				}
			});
	}

	pub fn syncing(&self) -> bool {
		self.syncer.is_initialized() && self.syncer.borrow().syncing()
	}

	/// Prepare options for the chain pipeline
	fn chain_opts(&self) -> chain::Options {
		let opts = if self.syncing() {
			chain::SYNC
		} else {
			chain::NONE
		};
		opts
	}
}

/// Implementation of the ChainAdapter for the network. Gets notified when the
/// blockchain accepted a new block, asking the pool to update its state and
/// the network to broadcast the block
pub struct ChainToPoolAndNetAdapter {
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	p2p: OneTime<Arc<Server>>,
}

impl ChainAdapter for ChainToPoolAndNetAdapter {
	fn block_accepted(&self, b: &core::Block) {
		{
			if let Err(e) = self.tx_pool.write().unwrap().reconcile_block(b) {
				error!(
					LOGGER,
					"Pool could not update itself at block {}: {:?}",
					b.hash(),
					e
				);
			}
		}
		self.p2p.borrow().broadcast_block(b);
	}
}

impl ChainToPoolAndNetAdapter {
	pub fn new(
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	) -> ChainToPoolAndNetAdapter {
		ChainToPoolAndNetAdapter {
			tx_pool: tx_pool,
			p2p: OneTime::new(),
		}
	}
	pub fn init(&self, p2p: Arc<Server>) {
		self.p2p.init(p2p);
	}
}

/// Adapter between the transaction pool and the network, to relay
/// transactions that have been accepted.
pub struct PoolToNetAdapter {
	p2p: OneTime<Arc<Server>>,
}

impl pool::PoolAdapter for PoolToNetAdapter {
	fn tx_accepted(&self, tx: &core::Transaction) {
		self.p2p.borrow().broadcast_transaction(tx);
	}
}

impl PoolToNetAdapter {
	/// Create a new pool to net adapter
	pub fn new() -> PoolToNetAdapter {
		PoolToNetAdapter {
			p2p: OneTime::new(),
		}
	}

	/// Setup the p2p server on the adapter
	pub fn init(&self, p2p: Arc<Server>) {
		self.p2p.init(p2p);
	}
}

/// Implements the view of the blockchain required by the TransactionPool to
/// operate. Mostly needed to break any direct lifecycle or implementation
/// dependency between the pool and the chain.
#[derive(Clone)]
pub struct PoolToChainAdapter {
	chain: OneTime<Arc<chain::Chain>>,
}

impl PoolToChainAdapter {
	/// Create a new pool adapter
	pub fn new() -> PoolToChainAdapter {
		PoolToChainAdapter {
			chain: OneTime::new(),
		}
	}

	pub fn set_chain(&self, chain_ref: Arc<chain::Chain>) {
		self.chain.init(chain_ref);
	}
}

impl pool::BlockChain for PoolToChainAdapter {
	fn get_unspent(&self, output_ref: &Commitment) -> Result<Output, pool::PoolError> {
		self.chain
			.borrow()
			.get_unspent(output_ref)
			.map_err(|e| match e {
				chain::types::Error::OutputNotFound => pool::PoolError::OutputNotFound,
				chain::types::Error::OutputSpent => pool::PoolError::OutputSpent,
				_ => pool::PoolError::GenericPoolError,
			})
	}

	fn get_block_header_by_output_commit(
		&self,
		commit: &Commitment,
	) -> Result<BlockHeader, pool::PoolError> {
		self.chain
			.borrow()
			.get_block_header_by_output_commit(commit)
			.map_err(|_| pool::PoolError::GenericPoolError)
	}

	fn head_header(&self) -> Result<BlockHeader, pool::PoolError> {
		self.chain
			.borrow()
			.head_header()
			.map_err(|_| pool::PoolError::GenericPoolError)
	}
}
