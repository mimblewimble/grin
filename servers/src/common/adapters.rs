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

//! Adapters connecting new block, new transaction, and accepted transaction
//! events to consumers of those events.

use crate::util::RwLock;
use std::fs::File;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::Instant;

use crate::chain::{self, BlockStatus, ChainAdapter, Options};
use crate::common::types::{self, ChainValidationMode, ServerConfig, SyncState, SyncStatus};
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::transaction::Transaction;
use crate::core::core::verifier_cache::VerifierCache;
use crate::core::core::{BlockHeader, BlockSums, CompactBlock};
use crate::core::pow::Difficulty;
use crate::core::{core, global};
use crate::p2p;
use crate::pool;
use crate::util::OneTime;
use chrono::prelude::*;
use chrono::Duration;
use rand::prelude::*;

/// Implementation of the NetAdapter for the . Gets notified when new
/// blocks and transactions are received and forwards to the chain and pool
/// implementations.
pub struct NetToChainAdapter {
	sync_state: Arc<SyncState>,
	chain: Weak<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	peers: OneTime<Weak<p2p::Peers>>,
	config: ServerConfig,
}

impl p2p::ChainAdapter for NetToChainAdapter {
	fn total_difficulty(&self) -> Difficulty {
		self.chain().head().unwrap().total_difficulty
	}

	fn total_height(&self) -> u64 {
		self.chain().head().unwrap().height
	}

	fn get_transaction(&self, kernel_hash: Hash) -> Option<core::Transaction> {
		self.tx_pool.read().retrieve_tx_by_kernel_hash(kernel_hash)
	}

	fn tx_kernel_received(&self, kernel_hash: Hash, addr: SocketAddr) {
		// nothing much we can do with a new transaction while syncing
		if self.sync_state.is_syncing() {
			return;
		}

		let tx = self.tx_pool.read().retrieve_tx_by_kernel_hash(kernel_hash);

		if tx.is_none() {
			self.request_transaction(kernel_hash, &addr);
		}
	}

	fn transaction_received(&self, tx: core::Transaction, stem: bool) {
		// nothing much we can do with a new transaction while syncing
		if self.sync_state.is_syncing() {
			return;
		}

		let source = pool::TxSource {
			debug_name: "p2p".to_string(),
			identifier: "?.?.?.?".to_string(),
		};

		let tx_hash = tx.hash();
		let header = self.chain().head_header().unwrap();

		debug!(
			"Received tx {}, inputs: {}, outputs: {}, kernels: {}, going to process.",
			tx_hash,
			tx.inputs().len(),
			tx.outputs().len(),
			tx.kernels().len(),
		);

		let res = {
			let mut tx_pool = self.tx_pool.write();
			tx_pool.add_to_pool(source, tx, stem, &header)
		};

		if let Err(e) = res {
			debug!("Transaction {} rejected: {:?}", tx_hash, e);
		}
	}

	fn block_received(&self, b: core::Block, addr: SocketAddr) -> bool {
		debug!(
			"Received block {} at {} from {}, inputs: {}, outputs: {}, kernels: {}, going to process.",
			b.hash(),
			b.header.height,
			addr,
			b.inputs().len(),
			b.outputs().len(),
			b.kernels().len(),
		);
		self.process_block(b, addr)
	}

	fn compact_block_received(&self, cb: core::CompactBlock, addr: SocketAddr) -> bool {
		let bhash = cb.hash();
		debug!(
			"Received compact_block {} at {} from {}, outputs: {}, kernels: {}, kern_ids: {}, going to process.",
			bhash,
			cb.header.height,
			addr,
			cb.out_full().len(),
			cb.kern_full().len(),
			cb.kern_ids().len(),
		);

		let cb_hash = cb.hash();
		if cb.kern_ids().is_empty() {
			// push the freshly hydrated block through the chain pipeline
			match core::Block::hydrate_from(cb, vec![]) {
				Ok(block) => self.process_block(block, addr),
				Err(e) => {
					debug!("Invalid hydrated block {}: {}", cb_hash, e);
					return false;
				}
			}
		} else {
			// check at least the header is valid before hydrating
			if let Err(e) = self
				.chain()
				.process_block_header(&cb.header, self.chain_opts())
			{
				debug!("Invalid compact block header {}: {}", cb_hash, e);
				return !e.is_bad_data();
			}

			let (txs, missing_short_ids) = {
				self.tx_pool
					.read()
					.retrieve_transactions(cb.hash(), cb.nonce, cb.kern_ids())
			};

			debug!(
				"adapter: txs from tx pool - {}, (unknown kern_ids: {})",
				txs.len(),
				missing_short_ids.len(),
			);

			// TODO - 3 scenarios here -
			// 1) we hydrate a valid block (good to go)
			// 2) we hydrate an invalid block (txs legit missing from our pool)
			// 3) we hydrate an invalid block (peer sent us a "bad" compact block) - [TBD]

			let block = match core::Block::hydrate_from(cb.clone(), txs) {
				Ok(block) => block,
				Err(e) => {
					debug!("Invalid hydrated block {}: {}", cb.hash(), e);
					return false;
				}
			};

			if let Ok(prev) = self.chain().get_previous_header(&cb.header) {
				if block
					.validate(&prev.total_kernel_offset, self.verifier_cache.clone())
					.is_ok()
				{
					debug!("successfully hydrated block from tx pool!");
					self.process_block(block, addr)
				} else {
					if self.sync_state.status() == SyncStatus::NoSync {
						debug!("adapter: block invalid after hydration, requesting full block");
						self.request_block(&cb.header, &addr);
						true
					} else {
						debug!("block invalid after hydration, ignoring it, cause still syncing");
						true
					}
				}
			} else {
				debug!("failed to retrieve previous block header (still syncing?)");
				true
			}
		}
	}

	fn header_received(&self, bh: core::BlockHeader, addr: SocketAddr) -> bool {
		let bhash = bh.hash();
		debug!(
			"Received block header {} at {} from {}, going to process.",
			bhash, bh.height, addr,
		);

		// pushing the new block header through the header chain pipeline
		// we will go ask for the block if this is a new header
		let res = self.chain().process_block_header(&bh, self.chain_opts());

		if let &Err(ref e) = &res {
			debug!("Block header {} refused by chain: {:?}", bhash, e.kind());
			if e.is_bad_data() {
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

	fn headers_received(&self, bhs: &[core::BlockHeader], addr: SocketAddr) -> bool {
		info!(
			"Received block headers {:?} from {}",
			bhs.iter().map(|x| x.hash()).collect::<Vec<_>>(),
			addr,
		);

		if bhs.len() == 0 {
			return false;
		}

		// try to add headers to our header chain
		let res = self.chain().sync_block_headers(bhs, self.chain_opts());
		if let &Err(ref e) = &res {
			debug!("Block headers refused by chain: {:?}", e);

			if e.is_bad_data() {
				return false;
			}
		}
		true
	}

	fn locate_headers(&self, locator: &[Hash]) -> Vec<core::BlockHeader> {
		debug!("locator: {:?}", locator);

		let header = match self.find_common_header(locator) {
			Some(header) => header,
			None => return vec![],
		};

		let max_height = self.chain().header_head().unwrap().height;

		// looks like we know one, getting as many following headers as allowed
		let hh = header.height;
		let mut headers = vec![];
		for h in (hh + 1)..=(hh + (p2p::MAX_BLOCK_HEADERS as u64)) {
			if h > max_height {
				break;
			}

			if let Ok(header) = self.chain().get_header_by_height(h) {
				headers.push(header);
			} else {
				error!("Failed to locate headers successfully.");
				break;
			}
		}

		debug!("returning headers: {}", headers.len());

		headers
	}

	/// Gets a full block by its hash.
	fn get_block(&self, h: Hash) -> Option<core::Block> {
		let b = self.chain().get_block(&h);
		match b {
			Ok(b) => Some(b),
			_ => None,
		}
	}

	/// Provides a reading view into the current txhashset state as well as
	/// the required indexes for a consumer to rewind to a consistent state
	/// at the provided block hash.
	fn txhashset_read(&self, h: Hash) -> Option<p2p::TxHashSetRead> {
		match self.chain().txhashset_read(h.clone()) {
			Ok((out_index, kernel_index, read)) => Some(p2p::TxHashSetRead {
				output_index: out_index,
				kernel_index: kernel_index,
				reader: read,
			}),
			Err(e) => {
				warn!("Couldn't produce txhashset data for block {}: {:?}", h, e);
				None
			}
		}
	}

	fn txhashset_receive_ready(&self) -> bool {
		match self.sync_state.status() {
			SyncStatus::TxHashsetDownload { .. } => true,
			_ => false,
		}
	}

	fn txhashset_download_update(
		&self,
		start_time: DateTime<Utc>,
		downloaded_size: u64,
		total_size: u64,
	) -> bool {
		match self.sync_state.status() {
			SyncStatus::TxHashsetDownload { .. } => {
				self.sync_state
					.update_txhashset_download(SyncStatus::TxHashsetDownload {
						start_time,
						downloaded_size,
						total_size,
					})
			}
			_ => false,
		}
	}

	/// Writes a reading view on a txhashset state that's been provided to us.
	/// If we're willing to accept that new state, the data stream will be
	/// read as a zip file, unzipped and the resulting state files should be
	/// rewound to the provided indexes.
	fn txhashset_write(&self, h: Hash, txhashset_data: File, _peer_addr: SocketAddr) -> bool {
		// check status again after download, in case 2 txhashsets made it somehow
		if let SyncStatus::TxHashsetDownload { .. } = self.sync_state.status() {
		} else {
			return true;
		}

		if let Err(e) = self
			.chain()
			.txhashset_write(h, txhashset_data, self.sync_state.as_ref())
		{
			error!("Failed to save txhashset archive: {}", e);
			let is_good_data = !e.is_bad_data();
			self.sync_state.set_sync_error(types::Error::Chain(e));
			is_good_data
		} else {
			info!("Received valid txhashset data for {}.", h);
			true
		}
	}
}

impl NetToChainAdapter {
	/// Construct a new NetToChainAdapter instance
	pub fn new(
		sync_state: Arc<SyncState>,
		chain: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool>>,
		verifier_cache: Arc<RwLock<dyn VerifierCache>>,
		config: ServerConfig,
	) -> NetToChainAdapter {
		NetToChainAdapter {
			sync_state,
			chain: Arc::downgrade(&chain),
			tx_pool,
			verifier_cache,
			peers: OneTime::new(),
			config,
		}
	}

	/// Initialize a NetToChainAdaptor with reference to a Peers object.
	/// Should only be called once.
	pub fn init(&self, peers: Arc<p2p::Peers>) {
		self.peers.init(Arc::downgrade(&peers));
	}

	fn peers(&self) -> Arc<p2p::Peers> {
		self.peers
			.borrow()
			.upgrade()
			.expect("Failed to upgrade weak ref to our peers.")
	}

	fn chain(&self) -> Arc<chain::Chain> {
		self.chain
			.upgrade()
			.expect("Failed to upgrade weak ref to our chain.")
	}

	// Find the first locator hash that refers to a known header on our main chain.
	fn find_common_header(&self, locator: &[Hash]) -> Option<BlockHeader> {
		for hash in locator {
			if let Ok(header) = self.chain().get_block_header(&hash) {
				if let Ok(header_at_height) = self.chain().get_header_by_height(header.height) {
					if header.hash() == header_at_height.hash() {
						return Some(header);
					}
				}
			}
		}
		None
	}

	// pushing the new block through the chain pipeline
	// remembering to reset the head if we have a bad block
	fn process_block(&self, b: core::Block, addr: SocketAddr) -> bool {
		// We cannot process blocks earlier than the horizon so check for this here.
		{
			let head = self.chain().head().unwrap();
			let horizon = head
				.height
				.saturating_sub(global::cut_through_horizon() as u64);
			if b.header.height < horizon {
				return true;
			}
		}

		let bhash = b.hash();
		let previous = self.chain().get_previous_header(&b.header);

		match self.chain().process_block(b, self.chain_opts()) {
			Ok(_) => {
				self.validate_chain(bhash);
				self.check_compact();
				true
			}
			Err(ref e) if e.is_bad_data() => {
				self.validate_chain(bhash);
				false
			}
			Err(e) => {
				match e.kind() {
					chain::ErrorKind::Orphan => {
						if let Ok(previous) = previous {
							// make sure we did not miss the parent block
							if !self.chain().is_orphan(&previous.hash())
								&& !self.sync_state.is_syncing()
							{
								debug!("process_block: received an orphan block, checking the parent: {:}", previous.hash());
								self.request_block_by_hash(previous.hash(), &addr)
							}
						}
						true
					}
					_ => {
						debug!(
							"process_block: block {} refused by chain: {}",
							bhash,
							e.kind()
						);
						true
					}
				}
			}
		}
	}

	fn validate_chain(&self, bhash: Hash) {
		// If we are running in "validate the full chain every block" then
		// panic here if validation fails for any reason.
		// We are out of consensus at this point and want to track the problem
		// down as soon as possible.
		// Skip this if we are currently syncing (too slow).
		if self.config.chain_validation_mode == ChainValidationMode::EveryBlock
			&& self.chain().head().unwrap().height > 0
			&& !self.sync_state.is_syncing()
		{
			let now = Instant::now();

			debug!(
				"process_block: ***** validating full chain state at {}",
				bhash,
			);

			self.chain()
				.validate(true)
				.expect("chain validation failed, hard stop");

			debug!(
				"process_block: ***** done validating full chain state, took {}s",
				now.elapsed().as_secs(),
			);
		}
	}

	fn check_compact(&self) {
		// Skip compaction if we are syncing.
		if self.sync_state.is_syncing() {
			return;
		}

		// Roll the dice to trigger compaction at 1/COMPACTION_CHECK chance per block,
		// uses a different thread to avoid blocking the caller thread (likely a peer)
		let mut rng = thread_rng();
		if 0 == rng.gen_range(0, global::COMPACTION_CHECK) {
			let chain = self.chain().clone();
			let _ = thread::Builder::new()
				.name("compactor".to_string())
				.spawn(move || {
					if let Err(e) = chain.compact() {
						error!("Could not compact chain: {:?}", e);
					}
				});
		}
	}

	fn request_transaction(&self, h: Hash, addr: &SocketAddr) {
		self.send_tx_request_to_peer(h, addr, |peer, h| peer.send_tx_request(h))
	}

	// After receiving a compact block if we cannot successfully hydrate
	// it into a full block then fallback to requesting the full block
	// from the same peer that gave us the compact block
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
	fn request_compact_block(&self, bh: &BlockHeader, addr: &SocketAddr) {
		self.send_block_request_to_peer(bh.hash(), addr, |peer, h| {
			peer.send_compact_block_request(h)
		})
	}

	fn send_tx_request_to_peer<F>(&self, h: Hash, addr: &SocketAddr, f: F)
	where
		F: Fn(&p2p::Peer, Hash) -> Result<(), p2p::Error>,
	{
		match self.peers().get_connected_peer(addr) {
			None => debug!(
				"send_tx_request_to_peer: can't send request to peer {:?}, not connected",
				addr
			),
			Some(peer) => {
				if let Err(e) = f(&peer, h) {
					error!("send_tx_request_to_peer: failed: {:?}", e)
				}
			}
		}
	}

	fn send_block_request_to_peer<F>(&self, h: Hash, addr: &SocketAddr, f: F)
	where
		F: Fn(&p2p::Peer, Hash) -> Result<(), p2p::Error>,
	{
		match self.chain().block_exists(h) {
			Ok(false) => match self.peers().get_connected_peer(addr) {
				None => debug!(
					"send_block_request_to_peer: can't send request to peer {:?}, not connected",
					addr
				),
				Some(peer) => {
					if let Err(e) = f(&peer, h) {
						error!("send_block_request_to_peer: failed: {:?}", e)
					}
				}
			},
			Ok(true) => debug!("send_block_request_to_peer: block {} already known", h),
			Err(e) => error!(
				"send_block_request_to_peer: failed to check block exists: {:?}",
				e
			),
		}
	}

	/// Prepare options for the chain pipeline
	fn chain_opts(&self) -> chain::Options {
		let opts = if self.sync_state.is_syncing() {
			chain::Options::SYNC
		} else {
			chain::Options::NONE
		};
		opts
	}
}

/// Implementation of the ChainAdapter for the network. Gets notified when the
///  accepted a new block, asking the pool to update its state and
/// the network to broadcast the block
pub struct ChainToPoolAndNetAdapter {
	sync_state: Arc<SyncState>,
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	peers: OneTime<Weak<p2p::Peers>>,
}

impl ChainAdapter for ChainToPoolAndNetAdapter {
	fn block_accepted(&self, b: &core::Block, status: BlockStatus, opts: Options) {
		match status {
			BlockStatus::Reorg => {
				warn!(
					"block_accepted (REORG!): {:?} at {} (diff: {})",
					b.hash(),
					b.header.height,
					b.header.total_difficulty(),
				);
			}
			BlockStatus::Fork => {
				debug!(
					"block_accepted (fork?): {:?} at {} (diff: {})",
					b.hash(),
					b.header.height,
					b.header.total_difficulty(),
				);
			}
			BlockStatus::Next => {
				debug!(
					"block_accepted (head+): {:?} at {} (diff: {})",
					b.hash(),
					b.header.height,
					b.header.total_difficulty(),
				);
			}
		}

		if self.sync_state.is_syncing() {
			return;
		}

		// If we mined the block then we want to broadcast the compact block.
		// If we received the block from another node then broadcast "header first"
		// to minimize network traffic.
		if opts.contains(Options::MINE) {
			// propagate compact block out if we mined the block
			let cb: CompactBlock = b.clone().into();
			self.peers().broadcast_compact_block(&cb);
		} else {
			// "header first" propagation if we are not the originator of this block
			self.peers().broadcast_header(&b.header);
		}

		// Reconcile the txpool against the new block *after* we have broadcast it too our peers.
		// This may be slow and we do not want to delay block propagation.
		// We only want to reconcile the txpool against the new block *if* total work has increased.
		if status == BlockStatus::Next || status == BlockStatus::Reorg {
			let mut tx_pool = self.tx_pool.write();

			let _ = tx_pool.reconcile_block(b);

			// First "age out" any old txs in the reorg_cache.
			let cutoff = Utc::now() - Duration::minutes(30);
			tx_pool.truncate_reorg_cache(cutoff);
		}

		if status == BlockStatus::Reorg {
			let _ = self.tx_pool.write().reconcile_reorg_cache(&b.header);
		}
	}
}

impl ChainToPoolAndNetAdapter {
	/// Construct a ChainToPoolAndNetAdapter instance.
	pub fn new(
		sync_state: Arc<SyncState>,
		tx_pool: Arc<RwLock<pool::TransactionPool>>,
	) -> ChainToPoolAndNetAdapter {
		ChainToPoolAndNetAdapter {
			sync_state,
			tx_pool,
			peers: OneTime::new(),
		}
	}

	/// Initialize a ChainToPoolAndNetAdapter instance with handle to a Peers
	/// object. Should only be called once.
	pub fn init(&self, peers: Arc<p2p::Peers>) {
		self.peers.init(Arc::downgrade(&peers));
	}

	fn peers(&self) -> Arc<p2p::Peers> {
		self.peers
			.borrow()
			.upgrade()
			.expect("Failed to upgrade weak ref to our peers.")
	}
}

/// Adapter between the transaction pool and the network, to relay
/// transactions that have been accepted.
pub struct PoolToNetAdapter {
	peers: OneTime<Weak<p2p::Peers>>,
}

impl pool::PoolAdapter for PoolToNetAdapter {
	fn stem_tx_accepted(&self, tx: &core::Transaction) -> Result<(), pool::PoolError> {
		self.peers()
			.relay_stem_transaction(tx)
			.map_err(|_| pool::PoolError::DandelionError)?;
		Ok(())
	}

	fn tx_accepted(&self, tx: &core::Transaction) {
		self.peers().broadcast_transaction(tx);
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
	pub fn init(&self, peers: Arc<p2p::Peers>) {
		self.peers.init(Arc::downgrade(&peers));
	}

	fn peers(&self) -> Arc<p2p::Peers> {
		self.peers
			.borrow()
			.upgrade()
			.expect("Failed to upgrade weak ref to our peers.")
	}
}

/// Implements the view of the  required by the TransactionPool to
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

	/// Set the pool adapter's chain. Should only be called once.
	pub fn set_chain(&self, chain_ref: Arc<chain::Chain>) {
		self.chain.init(Arc::downgrade(&chain_ref));
	}

	fn chain(&self) -> Arc<chain::Chain> {
		self.chain
			.borrow()
			.upgrade()
			.expect("Failed to upgrade the weak ref to our chain.")
	}
}

impl pool::BlockChain for PoolToChainAdapter {
	fn chain_head(&self) -> Result<BlockHeader, pool::PoolError> {
		self.chain()
			.head_header()
			.map_err(|_| pool::PoolError::Other(format!("failed to get head_header")))
	}

	fn get_block_header(&self, hash: &Hash) -> Result<BlockHeader, pool::PoolError> {
		self.chain()
			.get_block_header(hash)
			.map_err(|_| pool::PoolError::Other(format!("failed to get block_header")))
	}

	fn get_block_sums(&self, hash: &Hash) -> Result<BlockSums, pool::PoolError> {
		self.chain()
			.get_block_sums(hash)
			.map_err(|_| pool::PoolError::Other(format!("failed to get block_sums")))
	}

	fn validate_tx(&self, tx: &Transaction) -> Result<(), pool::PoolError> {
		self.chain()
			.validate_tx(tx)
			.map_err(|_| pool::PoolError::Other(format!("failed to validate tx")))
	}

	fn verify_coinbase_maturity(&self, tx: &Transaction) -> Result<(), pool::PoolError> {
		self.chain()
			.verify_coinbase_maturity(tx)
			.map_err(|_| pool::PoolError::ImmatureCoinbase)
	}

	fn verify_tx_lock_height(&self, tx: &Transaction) -> Result<(), pool::PoolError> {
		self.chain()
			.verify_tx_lock_height(tx)
			.map_err(|_| pool::PoolError::ImmatureTransaction)
	}
}
