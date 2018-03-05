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
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::{Arc, RwLock, Weak};
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

// All adapters use `Weak` references instead of `Arc` to avoid cycles that
// can never be destroyed. These 2 functions are simple helpers to reduce the
// boilerplate of dealing with `Weak`.
fn w<T>(weak: &Weak<T>) -> Arc<T> {
	weak.upgrade().unwrap()
}

fn wo<T>(weak_one: &OneTime<Weak<T>>) -> Arc<T> {
	w(weak_one.borrow().deref())
}

/// Implementation of the NetAdapter for the blockchain. Gets notified when new
/// blocks and transactions are received and forwards to the chain and pool
/// implementations.
pub struct NetToChainAdapter {
	currently_syncing: Arc<AtomicBool>,
	chain: Weak<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	peers: OneTime<Weak<p2p::Peers>>,
}

impl p2p::ChainAdapter for NetToChainAdapter {
	fn total_difficulty(&self) -> Difficulty {
		w(&self.chain).total_difficulty()
	}

	fn total_height(&self) -> u64 {
		w(&self.chain).head().unwrap().height
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
		self.process_block(b, addr)
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
			let block = core::Block::hydrate_from(cb, vec![]);

			// push the freshly hydrated block through the chain pipeline
			self.process_block(block, addr)
		} else {
			// TODO - do we need to validate the header here?

			let txs = {
				let tx_pool = self.tx_pool.read().unwrap();
				tx_pool.retrieve_transactions(&cb)
			};

			debug!(LOGGER, "adapter: txs from tx pool - {}", txs.len(),);

			// TODO - 3 scenarios here -
			// 1) we hydrate a valid block (good to go)
			// 2) we hydrate an invalid block (txs legit missing from our pool)
			// 3) we hydrate an invalid block (peer sent us a "bad" compact block) - [TBD]

			let block = core::Block::hydrate_from(cb.clone(), txs);

			if let Ok(()) = block.validate() {
				debug!(LOGGER, "adapter: successfully hydrated block from tx pool!");
				self.process_block(block, addr)
			} else {
				debug!(
					LOGGER,
					"adapter: block invalid after hydration, requesting full block"
				);
				self.request_block(&cb.header, &addr);
				true
			}
		}
	}

	fn header_received(&self, bh: core::BlockHeader, addr: SocketAddr) -> bool {
		let bhash = bh.hash();
		debug!(
			LOGGER,
			"Received block header {} at {} from {}, going to process.", bhash, bh.height, addr,
		);

		// pushing the new block header through the header chain pipeline
		// we will go ask for the block if this is a new header
		let res = w(&self.chain).process_block_header(&bh, self.chain_opts());

		if let &Err(ref e) = &res {
			debug!(LOGGER, "Block header {} refused by chain: {:?}", bhash, e);
			if e.is_bad_data() {
				debug!(
					LOGGER,
					"header_received: {} is a bad header, resetting header head", bhash
				);
				let _ = w(&self.chain).reset_head();
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
			let res = w(&self.chain).sync_block_header(&bh, self.chain_opts());
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

		let header_head = w(&self.chain).get_header_head().unwrap();
		info!(
			LOGGER,
			"Added {} headers to the header chain. Last: {} at {}.",
			added_hs.len(),
			header_head.last_block_h,
			header_head.height,
		);
	}

	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader> {
		debug!(LOGGER, "locate_headers: {:?}", locator,);

		let header = match self.find_common_header(locator) {
			Some(header) => header,
			None => return vec![],
		};

		debug!(LOGGER, "locate_headers: common header: {:?}", header.hash(),);

		// looks like we know one, getting as many following headers as allowed
		let hh = header.height;
		let mut headers = vec![];
		for h in (hh + 1)..(hh + (p2p::MAX_BLOCK_HEADERS as u64)) {
			let header = w(&self.chain).get_header_by_height(h);
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
		let b = w(&self.chain).get_block(&h);
		match b {
			Ok(b) => Some(b),
			_ => None,
		}
	}

	/// Provides a reading view into the current txhashset state as well as
	/// the required indexes for a consumer to rewind to a consistant state
	/// at the provided block hash.
	fn txhashset_read(&self, h: Hash) -> Option<p2p::TxHashSetRead> {
		match w(&self.chain).txhashset_read(h.clone()) {
			Ok((out_index, kernel_index, read)) => Some(p2p::TxHashSetRead {
				output_index: out_index,
				kernel_index: kernel_index,
				reader: read,
			}),
			Err(e) => {
				warn!(
					LOGGER,
					"Couldn't produce txhashset data for block {}: {:?}", h, e
				);
				None
			}
		}
	}

	/// Writes a reading view on a txhashset state that's been provided to us.
	/// If we're willing to accept that new state, the data stream will be
	/// read as a zip file, unzipped and the resulting state files should be
	/// rewound to the provided indexes.
	fn txhashset_write(
		&self,
		h: Hash,
		rewind_to_output: u64,
		rewind_to_kernel: u64,
		txhashset_data: File,
		_peer_addr: SocketAddr,
	) -> bool {
		// TODO check whether we should accept any txhashset now
		if let Err(e) =
			w(&self.chain).txhashset_write(h, rewind_to_output, rewind_to_kernel, txhashset_data)
		{
			error!(LOGGER, "Failed to save txhashset archive: {:?}", e);
			!e.is_bad_data()
		} else {
			info!(LOGGER, "Received valid txhashset data for {}.", h);
			self.currently_syncing.store(true, Ordering::Relaxed);
			true
		}
	}
}

impl NetToChainAdapter {
	pub fn new(
		currently_syncing: Arc<AtomicBool>,
		chain_ref: Weak<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	) -> NetToChainAdapter {
		NetToChainAdapter {
			currently_syncing: currently_syncing,
			chain: chain_ref,
			tx_pool: tx_pool,
			peers: OneTime::new(),
		}
	}

	pub fn init(&self, peers: Weak<p2p::Peers>) {
		self.peers.init(peers);
	}

	// recursively go back through the locator vector and stop when we find
	// a header that we recognize this will be a header shared in common
	// between us and the peer
	fn find_common_header(&self, locator: Vec<Hash>) -> Option<BlockHeader> {
		if locator.len() == 0 {
			return None;
		}

		let chain = w(&self.chain);
		let known = chain.get_block_header(&locator[0]);

		match known {
			Ok(header) => {
				// even if we know the block, it may not be on our winning chain
				let known_winning = chain.get_header_by_height(header.height);
				if let Ok(known_winning) = known_winning {
					if known_winning.hash() != header.hash() {
						self.find_common_header(locator[1..].to_vec())
					} else {
						Some(header)
					}
				} else {
					self.find_common_header(locator[1..].to_vec())
				}
			}
			Err(chain::Error::StoreErr(store::Error::NotFoundErr, _)) => {
				self.find_common_header(locator[1..].to_vec())
			}
			Err(e) => {
				error!(LOGGER, "Could not build header locator: {:?}", e);
				None
			}
		}
	}

	// pushing the new block through the chain pipeline
	// remembering to reset the head if we have a bad block
	fn process_block(&self, b: core::Block, addr: SocketAddr) -> bool {
		let prev_hash = b.header.previous;
		let bhash = b.hash();
		let chain = w(&self.chain);
		match chain.process_block(b, self.chain_opts()) {
			Ok(_) => true,
			Err(chain::Error::Orphan) => {
				// make sure we did not miss the parent block
				if !self.currently_syncing.load(Ordering::Relaxed) && !chain.is_orphan(&prev_hash) {
					debug!(LOGGER, "adapter: process_block: received an orphan block, checking the parent: {:}", prev_hash);
					self.request_block_by_hash(prev_hash, &addr)
				}
				true
			}
			Err(ref e) if e.is_bad_data() => {
				debug!(
					LOGGER,
					"adapter: process_block: {} is a bad block, resetting head", bhash
				);
				let _ = chain.reset_head();
				false
			}
			Err(e) => {
				debug!(
					LOGGER,
					"adapter: process_block :block {} refused by chain: {:?}", bhash, e
				);
				true
			}
		}
	}

	// After receiving a compact block if we cannot successfully hydrate
	// it into a full block then fallback to requesting the full block
	// from the same peer that gave us the compact block
	//
	// TODO - currently only request block from a single peer
	// consider additional peers for redundancy?
	fn request_block(&self, bh: &BlockHeader, addr: &SocketAddr) {
		self.request_block_by_hash(bh.hash(), addr)
	}

	fn request_block_by_hash(&self, h: Hash, addr: &SocketAddr) {
		self.send_block_request_to_peer(h, addr, |peer, h| peer.send_block_request(h))
	}

	// After we have received a block header in "header first" propagation
	// we need to go request the block (compact representation) from the
	// same peer that gave us the header (unless we have already accepted the block)
	//
	// TODO - currently only request block from a single peer
	// consider additional peers for redundancy?
	fn request_compact_block(&self, bh: &BlockHeader, addr: &SocketAddr) {
		self.send_block_request_to_peer(bh.hash(), addr, |peer, h| {
			peer.send_compact_block_request(h)
		})
	}

	fn send_block_request_to_peer<F>(&self, h: Hash, addr: &SocketAddr, f: F)
	where
		F: Fn(&p2p::Peer, Hash) -> Result<(), p2p::Error>,
	{
		match w(&self.chain).block_exists(h) {
                        Ok(false) => {
                                match  wo(&self.peers).get_connected_peer(addr) {
                                        None => debug!(LOGGER, "send_block_request_to_peer: can't send request to peer {:?}, not connected", addr),
                                        Some(peer) => {
                                                match peer.read() {
                                                        Err(e) => debug!(LOGGER, "send_block_request_to_peer: can't send request to peer {:?}, read fails: {:?}", addr, e),
                                                        Ok(p) => {
                                                                if let Err(e) =  f(&p, h) {
                                                                        error!(LOGGER, "send_block_request_to_peer: failed: {:?}", e)
                                                                }
                                                        }
                                                }
                                        }
                                }
                        }
                        Ok(true) => debug!(LOGGER, "send_block_request_to_peer: block {} already known", h),
                        Err(e) => error!(LOGGER, "send_block_request_to_peer: failed to check block exists: {:?}", e)
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
	peers: OneTime<Weak<p2p::Peers>>,
}

impl ChainAdapter for ChainToPoolAndNetAdapter {
	fn block_accepted(&self, b: &core::Block, opts: Options) {
		debug!(LOGGER, "adapter: block_accepted: {:?}", b.hash());

		if let Err(e) = self.tx_pool.write().unwrap().reconcile_block(b) {
			error!(
				LOGGER,
				"Pool could not update itself at block {}: {:?}",
				b.hash(),
				e,
			);
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
					wo(&self.peers).broadcast_block(&b);
				} else {
					wo(&self.peers).broadcast_compact_block(&cb);
				}
			} else {
				wo(&self.peers).broadcast_compact_block(&cb);
			}
		} else {
			// "header first" propagation if we are not the originator of this block
			wo(&self.peers).broadcast_header(&b.header);
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
	pub fn init(&self, peers: Weak<p2p::Peers>) {
		self.peers.init(peers);
	}
}

/// Adapter between the transaction pool and the network, to relay
/// transactions that have been accepted.
pub struct PoolToNetAdapter {
	peers: OneTime<Weak<p2p::Peers>>,
}

impl pool::PoolAdapter for PoolToNetAdapter {
	fn tx_accepted(&self, tx: &core::Transaction) {
		wo(&self.peers).broadcast_transaction(tx);
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
	pub fn init(&self, peers: Weak<p2p::Peers>) {
		self.peers.init(peers);
	}
}

/// Implements the view of the blockchain required by the TransactionPool to
/// operate. Mostly needed to break any direct lifecycle or implementation
/// dependency between the pool and the chain.
#[derive(Clone)]
pub struct PoolToChainAdapter {
	chain: OneTime<Weak<chain::Chain>>,
}

impl PoolToChainAdapter {
	/// Create a new pool adapter
	pub fn new() -> PoolToChainAdapter {
		PoolToChainAdapter {
			chain: OneTime::new(),
		}
	}

	pub fn set_chain(&self, chain_ref: Weak<chain::Chain>) {
		self.chain.init(chain_ref);
	}
}

impl pool::BlockChain for PoolToChainAdapter {
	fn is_unspent(&self, output_ref: &OutputIdentifier) -> Result<Hash, pool::PoolError> {
		wo(&self.chain).is_unspent(output_ref).map_err(|e| match e {
			chain::types::Error::OutputNotFound => pool::PoolError::OutputNotFound,
			chain::types::Error::OutputSpent => pool::PoolError::OutputSpent,
			_ => pool::PoolError::GenericPoolError,
		})
	}

	fn is_matured(&self, input: &Input, height: u64) -> Result<(), pool::PoolError> {
		wo(&self.chain)
			.is_matured(input, height)
			.map_err(|e| match e {
				chain::types::Error::OutputNotFound => pool::PoolError::OutputNotFound,
				_ => pool::PoolError::GenericPoolError,
			})
	}

	fn head_header(&self) -> Result<BlockHeader, pool::PoolError> {
		wo(&self.chain)
			.head_header()
			.map_err(|_| pool::PoolError::GenericPoolError)
	}
}
