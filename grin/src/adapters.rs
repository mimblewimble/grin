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
use std::sync::atomic::{AtomicBool, Ordering};

use chain::{self, ChainAdapter};
use core::core::{self, Output};
use core::core::block::BlockHeader;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use p2p::{self, NetAdapter, PeerData, State};
use pool;
use util::secp::pedersen::Commitment;
use util::OneTime;
use store;
use util::LOGGER;

/// Implementation of the NetAdapter for the blockchain. Gets notified when new
/// blocks and transactions are received and forwards to the chain and pool
/// implementations.
pub struct NetToChainAdapter {
	chain: Arc<chain::Chain>,
	p2p_server: OneTime<Arc<p2p::Server>>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	syncing: AtomicBool,
}

impl NetAdapter for NetToChainAdapter {
	fn total_difficulty(&self) -> Difficulty {
		self.chain.total_difficulty()
	}

	fn total_height(&self) -> u64 {
		self.chain.head().unwrap().height
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

	fn block_received(&self, b: core::Block, addr: SocketAddr) {
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

			// if the peer sent us a block that's intrinsically bad, they're either
			// mistaken or manevolent, both of which require a ban
			if e.is_bad_block() {
				self.p2p_server.borrow().ban_peer(&addr);

				// and if we're currently syncing, our header chain is now wrong, it
				// needs to be reset
				if self.is_syncing() {
					let _ = self.chain.reset_header_head();
				}
			}
		}
	}

	fn headers_received(&self, bhs: Vec<core::BlockHeader>, addr: SocketAddr) {
		debug!(
			LOGGER,
			"Received block headers {:?} from {}",
			bhs.iter().map(|x| x.hash()).collect::<Vec<_>>(),
			addr
		);

		// try to add each header to our header chain
		let mut added_hs = vec![];
		for bh in bhs.clone() {
			let res = self.chain.process_block_header(&bh, self.chain_opts());
			match res {
				Ok(_) => {
					added_hs.push(bh);
				}
				Err(chain::Error::Unfit(s)) => {
					debug!(
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
		debug!(
			LOGGER,
			"Received {} headers for the header chain.",
			added_hs.len()
		);
	}

	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader> {
		debug!(
			LOGGER,
			"locate_headers: {:?}",
			locator,
		);

		let header = match self.find_common_header(locator) {
			Some(header) => header,
			None => return vec![],
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
		let peers = self.p2p_server.borrow()
			.find_peers(State::Healthy, capab, p2p::MAX_PEER_ADDRS as usize);
		debug!(LOGGER, "Got {} peer addrs to send.", peers.len());
		map_vec!(peers, |p| p.addr)
	}

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, peer_addrs: Vec<SocketAddr>) {
		debug!(LOGGER, "Received {} peer addrs, saving.", peer_addrs.len());
		for pa in peer_addrs {
			if let Ok(e) = self.p2p_server.borrow().exists_peer(pa) {
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
			if let Err(e) = self.p2p_server.borrow().save_peer(&peer) {
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
		if let Err(e) = self.p2p_server.borrow().save_peer(&peer) {
			error!(LOGGER, "Could not save connected peer: {:?}", e);
		}
	}

	fn peer_difficulty(&self, addr: SocketAddr, diff: Difficulty, height: u64) {
		debug!(
			LOGGER,
			"peer total_diff/height (ping/pong): {} , {}@{} vs us {}@{}",
			addr,
			diff,
			height,
			self.total_difficulty(),
			self.total_height()
		);

		if self.p2p_server.is_initialized() {
			if let Some(peer) = self.p2p_server.borrow().get_peer(&addr) {
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
	) -> NetToChainAdapter {
		NetToChainAdapter {
			chain: chain_ref,
			p2p_server: OneTime::new(),
			tx_pool: tx_pool,
			syncing: AtomicBool::new(true),
		}
	}

	/// Setup the p2p server on the adapter
	pub fn init(&self, p2p: Arc<p2p::Server>) {
		self.p2p_server.init(p2p);
	}

	/// Whether we're currently syncing the chain or we're fully caught up and
	/// just receiving blocks through gossip.
	pub fn is_syncing(&self) -> bool {
		let local_diff = self.total_difficulty();
		let peers = self.p2p_server.borrow().connected_peers();

		// if we're already syncing, we're caught up if no peer has a higher
		// difficulty than us
		if self.syncing.load(Ordering::Relaxed) {
			let higher_diff = peers.iter().any(|p| {
				let p = p.read().unwrap();
				p.info.total_difficulty > local_diff
			});
			if !higher_diff {
				info!(LOGGER, "sync: caught up on the most worked chain, disabling sync");
				self.syncing.store(false, Ordering::Relaxed);
			}
		} else {
			// if we're not syncing, we need to if our difficulty is much too low
			let higher_diff_padded = peers.iter().any(|p| {
				let p = p.read().unwrap();
				p.info.total_difficulty > local_diff.clone() + Difficulty::from_num(1000)
			});
			if higher_diff_padded {
				info!(LOGGER, "sync: late on the most worked chain, enabling sync");
				self.syncing.store(true, Ordering::Relaxed);
			}
		}
		self.syncing.load(Ordering::Relaxed)
	}
	
	// recursively go back through the locator vector and stop when we find
	// a header that we recognize this will be a header shared in common
	// between us and the peer
	fn find_common_header(&self, locator: Vec<Hash>) -> Option<BlockHeader> {
		if locator.len() == 0 {
			return None;
		}

		let known = self.chain.get_block_header(&locator[0]);

		match known {
			Ok(header) => {
				// even if we know the block, it may not be on our winning chain
				let known_winning = self.chain.get_header_by_height(header.height);
				if let Ok(known_winning) = known_winning {
					if known_winning.hash() != header.hash() {
						self.find_common_header(locator[1..].to_vec())
					} else {
						Some(header)
					}
				} else {
					self.find_common_header(locator[1..].to_vec())
				}
			},
			Err(chain::Error::StoreErr(store::Error::NotFoundErr, _)) => {
				self.find_common_header(locator[1..].to_vec())
			},
			Err(e) => {
				error!(LOGGER, "Could not build header locator: {:?}", e);
				None
			}
		}
	}

	/// Prepare options for the chain pipeline
	fn chain_opts(&self) -> chain::Options {
		let opts = if self.is_syncing() {
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
	p2p: OneTime<Arc<p2p::Server>>,
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
	pub fn init(&self, p2p: Arc<p2p::Server>) {
		self.p2p.init(p2p);
	}
}

/// Adapter between the transaction pool and the network, to relay
/// transactions that have been accepted.
pub struct PoolToNetAdapter {
	p2p: OneTime<Arc<p2p::Server>>,
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
	pub fn init(&self, p2p: Arc<p2p::Server>) {
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
