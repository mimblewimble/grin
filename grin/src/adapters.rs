// Copyright 2017 The Grin Developers
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
use rand;
use rand::Rng;

use chain::{self, ChainAdapter, Options};
use core::core;
use core::core::block::BlockHeader;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::core::transaction::{Input, OutputIdentifier};
use p2p;
use pool;
use util::OneTime;
use store;
use util::LOGGER;

/// Implementation of the NetAdapter for the blockchain. Gets notified when new
/// blocks and transactions are received and forwards to the chain and pool
/// implementations.
pub struct NetToChainAdapter {
	currently_syncing: Arc<AtomicBool>,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	peers: OneTime<p2p::Peers>,
}

impl p2p::ChainAdapter for NetToChainAdapter {
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

	fn block_received(&self, b: core::Block, addr: SocketAddr) -> bool {
		debug!(
			LOGGER,
			"Received block {} at {} from {}, going to process.",
			b.hash(),
			b.header.height,
			addr,
		);

		self.process_block(b)
	}

	fn compact_block_received(&self, cb: core::CompactBlock, addr: SocketAddr) -> bool {
		let bhash = cb.hash();
		debug!(
			LOGGER,
			"Received compact_block {} at {} from {}, going to process.",
			bhash,
			cb.header.height,
			addr,
		);

		if cb.kern_ids.is_empty() {
			let block = core::Block::hydrate_from(cb, vec![], vec![], vec![]);

			// push the freshly hydrated block through the chain pipeline
			self.process_block(block)
		} else {
			// TODO - do we need to validate the header here to be sure it is not total garbage?

			debug!(
				LOGGER,
				"*** cannot hydrate non-empty compact block (not yet implemented), \
				falling back to requesting full block",
			);
			self.request_block(&cb.header, &addr);
			true
		}
	}

	fn header_received(&self, bh: core::BlockHeader, addr: SocketAddr) -> bool {
		let bhash = bh.hash();
		debug!(
			LOGGER,
			"Received block header {} at {} from {}, going to process.",
			bhash,
			bh.height,
			addr,
		);

		// pushing the new block header through the header chain pipeline
		// we will go ask for the block if this is a new header
		let res = self.chain.process_block_header(&bh, self.chain_opts());

		if let &Err(ref e) = &res {
			debug!(LOGGER, "Block header {} refused by chain: {:?}", bhash, e);
			if e.is_bad_block() {
				debug!(LOGGER, "header_received: {} is a bad header, resetting header head", bhash);
				let _ = self.chain.reset_head();
				return false;
			} else {
				// we got an error when trying to process the block header
				// but nothing serious enough to need to ban the peer upstream
				return true;
			}
		}

		// we have successfully processed a block header
		// so we can go request the block itself
		self.request_compact_block(&bh, &addr);

		// done receiving the header
		true
	}

	fn headers_received(&self, bhs: Vec<core::BlockHeader>, addr: SocketAddr) {
		info!(
			LOGGER,
			"Received block headers {:?} from {}",
			bhs.iter().map(|x| x.hash()).collect::<Vec<_>>(),
			addr,
		);

		// try to add each header to our header chain
		let mut added_hs = vec![];
		for bh in bhs {
			let res = self.chain.sync_block_header(&bh, self.chain_opts());
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

}

impl NetToChainAdapter {
	pub fn new(
		currently_syncing: Arc<AtomicBool>,
		chain_ref: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	) -> NetToChainAdapter {
		NetToChainAdapter {
			currently_syncing: currently_syncing,
			chain: chain_ref,
			tx_pool: tx_pool,
			peers: OneTime::new(),
		}
	}

	pub fn init(&self, peers: p2p::Peers) {
		self.peers.init(peers);
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

	// pushing the new block through the chain pipeline
	// remembering to reset the head if we have a bad block
	fn process_block(&self, b: core::Block) -> bool {
		let bhash = b.hash();
		let res = self.chain.process_block(b, self.chain_opts());
		if let Err(ref e) = res {
			debug!(LOGGER, "Block {} refused by chain: {:?}", bhash, e);
			if e.is_bad_block() {
				debug!(LOGGER, "adapter: process_block: {} is a bad block, resetting head", bhash);
				let _ = self.chain.reset_head();
				return false;
			}
		};
		true
	}

	// After receiving a compact block if we cannot successfully hydrate
	// it into a full block then fallback to requesting the full block
	// from the same peer that gave us the compact block
	//
	// TODO - currently only request block from a single peer
	// consider additional peers for redundancy?
	fn request_block(&self, bh: &BlockHeader, addr: &SocketAddr) {
		if let None = self.peers.borrow().adapter.get_block(bh.hash()) {
			if let Some(peer) = self.peers.borrow().get_connected_peer(addr) {
				if let Ok(peer) = peer.read() {
					let _ = peer.send_block_request(bh.hash());
				}
			}
		} else {
			debug!(LOGGER, "request_block: block {} already known", bh.hash());
		}
	}

	// After we have received a block header in "header first" propagation
	// we need to go request the block (compact representation) from the
	// same peer that gave us the header (unless we have already accepted the block)
	//
	// TODO - currently only request block from a single peer
	// consider additional peers for redundancy?
	fn request_compact_block(&self, bh: &BlockHeader, addr: &SocketAddr) {
		if let None = self.peers.borrow().adapter.get_block(bh.hash()) {
			if let Some(peer) = self.peers.borrow().get_connected_peer(addr) {
				if let Ok(peer) = peer.read() {
					let _ = peer.send_compact_block_request(bh.hash());
				}
			}
		} else {
			debug!(LOGGER, "request_compact_block: block {} already known", bh.hash());
		}
	}

	/// Prepare options for the chain pipeline
	fn chain_opts(&self) -> chain::Options {
		let opts = if self.currently_syncing.load(Ordering::Relaxed) {
			chain::Options::SYNC
		} else {
			chain::Options::NONE
		};
		opts
	}
}

/// Implementation of the ChainAdapter for the network. Gets notified when the
/// blockchain accepted a new block, asking the pool to update its state and
/// the network to broadcast the block
pub struct ChainToPoolAndNetAdapter {
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	peers: OneTime<p2p::Peers>,
}

impl ChainAdapter for ChainToPoolAndNetAdapter {
	fn block_accepted(&self, b: &core::Block, opts: Options) {
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

		// If we mined the block then we want to broadcast the block itself.
		// If block is empty then broadcast the block.
		// If block contains txs then broadcast the compact block.
		// If we received the block from another node then broadcast "header first"
		// to minimize network traffic.
		if opts.contains(Options::MINE) {
			// propagate compact block out if we mined the block
			// but broadcast full block if we have no txs
			let cb = b.as_compact_block();
			if cb.kern_ids.is_empty() {

				// in the interest of testing all code paths
				// randomly decide how we send an empty block out
				// TODO - lock this down once we are comfortable it works...

				let mut rng = rand::thread_rng();
				if rng.gen() {
					self.peers.borrow().broadcast_block(&b);
				} else {
					self.peers.borrow().broadcast_compact_block(&cb);
				}
			} else {
				self.peers.borrow().broadcast_compact_block(&cb);
			}
		} else {
			// "header first" propagation if we are not the originator of this block
			self.peers.borrow().broadcast_header(&b.header);
		}
	}
}

impl ChainToPoolAndNetAdapter {
	pub fn new(
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	) -> ChainToPoolAndNetAdapter {
		ChainToPoolAndNetAdapter {
			tx_pool: tx_pool,
			peers: OneTime::new(),
		}
	}
	pub fn init(&self, peers: p2p::Peers) {
		self.peers.init(peers);
	}
}

/// Adapter between the transaction pool and the network, to relay
/// transactions that have been accepted.
pub struct PoolToNetAdapter {
	peers: OneTime<p2p::Peers>,
}

impl pool::PoolAdapter for PoolToNetAdapter {
	fn tx_accepted(&self, tx: &core::Transaction) {
		self.peers.borrow().broadcast_transaction(tx);
	}
}

impl PoolToNetAdapter {
	/// Create a new pool to net adapter
	pub fn new() -> PoolToNetAdapter {
		PoolToNetAdapter {
			peers: OneTime::new(),
		}
	}

	/// Setup the p2p server on the adapter
	pub fn init(&self, peers: p2p::Peers) {
		self.peers.init(peers);
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
	fn is_unspent(&self, output_ref: &OutputIdentifier) -> Result<(), pool::PoolError> {
		self.chain
			.borrow()
			.is_unspent(output_ref)
			.map_err(|e| match e {
				chain::types::Error::OutputNotFound => pool::PoolError::OutputNotFound,
				chain::types::Error::OutputSpent => pool::PoolError::OutputSpent,
				_ => pool::PoolError::GenericPoolError,
			})
	}

	fn is_matured(&self, input: &Input, height: u64) -> Result<(), pool::PoolError> {
		self.chain
			.borrow()
			.is_matured(input, height)
			.map_err(|e| match e {
				chain::types::Error::OutputNotFound => pool::PoolError::OutputNotFound,
				_ => pool::PoolError::GenericPoolError,
			})
		}

	fn head_header(&self) -> Result<BlockHeader, pool::PoolError> {
		self.chain
			.borrow()
			.head_header()
			.map_err(|_| pool::PoolError::GenericPoolError)
	}
}
