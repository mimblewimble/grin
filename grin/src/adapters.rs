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

use chain::{self, ChainAdapter};
use core::core::{self, Output};
use core::core::block::BlockHeader;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use p2p;
use pool;
use util::secp::pedersen::Commitment;
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

	fn block_received(&self, b: core::Block, _: SocketAddr) -> bool {
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
			if e.is_bad_block() {
				debug!(LOGGER, "block_received: {} is a bad block, resetting head", bhash);
				let _ = self.chain.reset_head();
				return false;
			}
		}
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

		// recursively go back through the locator vector
		// and stop when we find a header that we recognize
		// this will be a header shared in common between us and the peer
		let header = match self.locate_common(locator) {
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
		}
	}

	// finds the latest header on our main chain in common with the locator
	fn locate_common(&self, locator: Vec<Hash>) -> Option<BlockHeader> {
		if locator.len() == 0 {
			return None;
		}
		// recursively go back through the locator vector
		// and stop when we find a header that we recognize
		// this will be a header shared in common between us and the peer
		let known = self.chain.get_block_header(&locator[0]);
		match known {
			Ok(header) => {
				// checking that the header is really on our main chain
				let at_height = self.chain.get_header_by_height(header.height);
				if let Ok(known_at_height) = at_height {
					if known_at_height.hash() == header.hash() {
						return Some(header)
					}
				}
				self.locate_common(locator[1..].to_vec())
			}
			Err(chain::Error::StoreErr(store::Error::NotFoundErr, _)) => {
				self.locate_common(locator[1..].to_vec())
			}
			Err(e) => {
				error!(LOGGER, "Could not build header locator: {:?}", e);
				None
			}
		}
	}


	/// Prepare options for the chain pipeline
	fn chain_opts(&self) -> chain::Options {
		let opts = if self.currently_syncing.load(Ordering::Relaxed) {
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
	peers: OneTime<p2p::Peers>,
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
		self.peers.borrow().broadcast_block(b);
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
