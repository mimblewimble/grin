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

//! Facade and handler for the rest of the blockchain implementation
//! and mostly the chain pipeline.

use std::collections::HashMap;
use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lmdb;
use lru_cache::LruCache;

use core::core::hash::{Hash, Hashed};
use core::core::merkle_proof::MerkleProof;
use core::core::verifier_cache::VerifierCache;
use core::core::{Block, BlockHeader, BlockSums, Output, OutputIdentifier, Transaction, TxKernel};
use core::global;
use core::pow;
use error::{Error, ErrorKind};
use grin_store::Error::NotFoundErr;
use pipe;
use store;
use txhashset;
use types::{ChainAdapter, NoStatus, Options, Tip, TxHashsetWriteStatus};
use util::secp::pedersen::{Commitment, RangeProof};
use util::LOGGER;

/// Orphan pool size is limited by MAX_ORPHAN_SIZE
pub const MAX_ORPHAN_SIZE: usize = 200;

/// When evicting, very old orphans are evicted first
const MAX_ORPHAN_AGE_SECS: u64 = 300;

/// Number of recent hashes we keep to de-duplicate block or header sends
const HASHES_CACHE_SIZE: usize = 200;

#[derive(Debug, Clone)]
struct Orphan {
	block: Block,
	opts: Options,
	added: Instant,
}

pub struct OrphanBlockPool {
	// blocks indexed by their hash
	orphans: RwLock<HashMap<Hash, Orphan>>,
	// additional index of height -> hash
	// so we can efficiently identify a child block (ex-orphan) after processing a block
	height_idx: RwLock<HashMap<u64, Vec<Hash>>>,
	// accumulated number of evicted block because of MAX_ORPHAN_SIZE limitation
	evicted: AtomicUsize,
}

impl OrphanBlockPool {
	fn new() -> OrphanBlockPool {
		OrphanBlockPool {
			orphans: RwLock::new(HashMap::new()),
			height_idx: RwLock::new(HashMap::new()),
			evicted: AtomicUsize::new(0),
		}
	}

	fn len(&self) -> usize {
		let orphans = self.orphans.read().unwrap();
		orphans.len()
	}

	fn len_evicted(&self) -> usize {
		self.evicted.load(Ordering::Relaxed)
	}

	fn add(&self, orphan: Orphan) {
		let mut orphans = self.orphans.write().unwrap();
		let mut height_idx = self.height_idx.write().unwrap();
		{
			let height_hashes = height_idx
				.entry(orphan.block.header.height)
				.or_insert(vec![]);
			height_hashes.push(orphan.block.hash());
			orphans.insert(orphan.block.hash(), orphan);
		}

		if orphans.len() > MAX_ORPHAN_SIZE {
			let old_len = orphans.len();

			// evict too old
			orphans.retain(|_, ref mut x| {
				x.added.elapsed() < Duration::from_secs(MAX_ORPHAN_AGE_SECS)
			});
			// evict too far ahead
			let mut heights = height_idx.keys().cloned().collect::<Vec<u64>>();
			heights.sort_unstable();
			for h in heights.iter().rev() {
				if let Some(hs) = height_idx.remove(h) {
					for h in hs {
						let _ = orphans.remove(&h);
					}
				}
				if orphans.len() < MAX_ORPHAN_SIZE {
					break;
				}
			}
			// cleanup index
			height_idx.retain(|_, ref mut xs| xs.iter().any(|x| orphans.contains_key(&x)));

			self.evicted
				.fetch_add(old_len - orphans.len(), Ordering::Relaxed);
		}
	}

	/// Get an orphan from the pool indexed by the hash of its parent, removing
	/// it at the same time, preventing clone
	fn remove_by_height(&self, height: &u64) -> Option<Vec<Orphan>> {
		let mut orphans = self.orphans.write().unwrap();
		let mut height_idx = self.height_idx.write().unwrap();
		height_idx
			.remove(height)
			.map(|hs| hs.iter().filter_map(|h| orphans.remove(h)).collect())
	}

	pub fn contains(&self, hash: &Hash) -> bool {
		let orphans = self.orphans.read().unwrap();
		orphans.contains_key(hash)
	}
}

/// Facade to the blockchain block processing pipeline and storage. Provides
/// the current view of the TxHashSet according to the chain state. Also
/// maintains locking for the pipeline to avoid conflicting processing.
pub struct Chain {
	db_root: String,
	store: Arc<store::ChainStore>,
	adapter: Arc<ChainAdapter>,
	orphans: Arc<OrphanBlockPool>,
	txhashset: Arc<RwLock<txhashset::TxHashSet>>,
	// Recently processed blocks to avoid double-processing
	block_hashes_cache: Arc<RwLock<LruCache<Hash, bool>>>,
	verifier_cache: Arc<RwLock<VerifierCache>>,
	// POW verification function
	pow_verifier: fn(&BlockHeader, u8) -> Result<(), pow::Error>,
	archive_mode: bool,
}

unsafe impl Sync for Chain {}
unsafe impl Send for Chain {}

impl Chain {
	/// Initializes the blockchain and returns a new Chain instance. Does a
	/// check on the current chain head to make sure it exists and creates one
	/// based on the genesis block if necessary.
	pub fn init(
		db_root: String,
		db_env: Arc<lmdb::Environment>,
		adapter: Arc<ChainAdapter>,
		genesis: Block,
		pow_verifier: fn(&BlockHeader, u8) -> Result<(), pow::Error>,
		verifier_cache: Arc<RwLock<VerifierCache>>,
		archive_mode: bool,
	) -> Result<Chain, Error> {
		let chain_store = store::ChainStore::new(db_env)?;

		let store = Arc::new(chain_store);

		// open the txhashset, creating a new one if necessary
		let mut txhashset = txhashset::TxHashSet::open(db_root.clone(), store.clone(), None)?;

		setup_head(genesis, store.clone(), &mut txhashset)?;

		let head = store.head()?;
		debug!(
			LOGGER,
			"Chain init: {} @ {} [{}]",
			head.total_difficulty.to_num(),
			head.height,
			head.last_block_h,
		);

		Ok(Chain {
			db_root: db_root,
			store: store,
			adapter: adapter,
			orphans: Arc::new(OrphanBlockPool::new()),
			txhashset: Arc::new(RwLock::new(txhashset)),
			pow_verifier,
			verifier_cache,
			block_hashes_cache: Arc::new(RwLock::new(LruCache::new(HASHES_CACHE_SIZE))),
			archive_mode,
		})
	}

	/// Processes a single block, then checks for orphans, processing
	/// those as well if they're found
	pub fn process_block(&self, b: Block, opts: Options) -> Result<Option<Tip>, Error> {
		let height = b.header.height;
		let res = self.process_block_single(b, opts);
		if res.is_ok() {
			self.check_orphans(height + 1);
		}
		res
	}

	/// Attempt to add a new block to the chain.
	/// Returns true if it has been added to the longest chain
	/// or false if it has added to a fork (or orphan?).
	fn process_block_single(&self, b: Block, opts: Options) -> Result<Option<Tip>, Error> {
		let maybe_new_head: Result<Option<Tip>, Error>;
		{
			let batch = self.store.batch()?;
			let mut txhashset = self.txhashset.write().unwrap();
			let mut ctx = self.new_ctx(opts, batch, &mut txhashset)?;

			maybe_new_head = pipe::process_block(&b, &mut ctx);
			if let Ok(_) = maybe_new_head {
				ctx.batch.commit()?;
			}
			// release the lock and let the batch go before post-processing
		}

		let add_to_hash_cache = |hash: Hash| {
			// only add to hash cache below if block is definitively accepted
			// or rejected
			let mut cache = self.block_hashes_cache.write().unwrap();
			cache.insert(hash, true);
		};

		match maybe_new_head {
			Ok(head) => {
				add_to_hash_cache(b.hash());

				// notifying other parts of the system of the update
				self.adapter.block_accepted(&b, opts);

				Ok(head)
			}
			Err(e) => {
				match e.kind() {
					ErrorKind::Orphan => {
						let block_hash = b.hash();
						let orphan = Orphan {
							block: b,
							opts: opts,
							added: Instant::now(),
						};

						&self.orphans.add(orphan);

						debug!(
							LOGGER,
							"process_block: orphan: {:?}, # orphans {}{}",
							block_hash,
							self.orphans.len(),
							if self.orphans.len_evicted() > 0 {
								format!(", # evicted {}", self.orphans.len_evicted())
							} else {
								String::new()
							},
						);
						Err(ErrorKind::Orphan.into())
					}
					ErrorKind::Unfit(ref msg) => {
						debug!(
							LOGGER,
							"Block {} at {} is unfit at this time: {}",
							b.hash(),
							b.header.height,
							msg
						);
						Err(ErrorKind::Unfit(msg.clone()).into())
					}
					_ => {
						info!(
							LOGGER,
							"Rejected block {} at {}: {:?}",
							b.hash(),
							b.header.height,
							e
						);
						add_to_hash_cache(b.hash());
						Err(ErrorKind::Other(format!("{:?}", e).to_owned()).into())
					}
				}
			}
		}
	}

	/// Process a block header received during "header first" propagation.
	pub fn process_block_header(&self, bh: &BlockHeader, opts: Options) -> Result<(), Error> {
		let batch = self.store.batch()?;
		let mut txhashset = self.txhashset.write().unwrap();
		let mut ctx = self.new_ctx(opts, batch, &mut txhashset)?;
		pipe::process_block_header(bh, &mut ctx)?;
		ctx.batch.commit()?;
		Ok(())
	}

	/// Attempt to add new headers to the header chain (or fork).
	/// This is only ever used during sync and is based on sync_head.
	/// We update header_head here if our total work increases.
	pub fn sync_block_headers(
		&self,
		headers: &Vec<BlockHeader>,
		opts: Options,
	) -> Result<(), Error> {
		let batch = self.store.batch()?;
		let mut txhashset = self.txhashset.write().unwrap();
		let mut ctx = self.new_ctx(opts, batch, &mut txhashset)?;

		pipe::sync_block_headers(headers, &mut ctx)?;
		ctx.batch.commit()?;

		Ok(())
	}

	fn new_ctx<'a>(
		&self,
		opts: Options,
		batch: store::Batch<'a>,
		txhashset: &'a mut txhashset::TxHashSet,
	) -> Result<pipe::BlockContext<'a>, Error> {
		Ok(pipe::BlockContext {
			opts,
			pow_verifier: self.pow_verifier,
			block_hashes_cache: self.block_hashes_cache.clone(),
			verifier_cache: self.verifier_cache.clone(),
			txhashset,
			batch,
			orphans: self.orphans.clone(),
		})
	}

	/// Check if hash is for a known orphan.
	pub fn is_orphan(&self, hash: &Hash) -> bool {
		self.orphans.contains(hash)
	}

	/// Get the OrphanBlockPool accumulated evicted number of blocks
	pub fn orphans_evicted_len(&self) -> usize {
		self.orphans.len_evicted()
	}

	/// Check for orphans, once a block is successfully added
	pub fn check_orphans(&self, mut height: u64) {
		let initial_height = height;

		// Is there an orphan in our orphans that we can now process?
		loop {
			trace!(
				LOGGER,
				"check_orphans: at {}, # orphans {}",
				height,
				self.orphans.len(),
			);

			let mut orphan_accepted = false;
			let mut height_accepted = height;

			if let Some(orphans) = self.orphans.remove_by_height(&height) {
				let orphans_len = orphans.len();
				for (i, orphan) in orphans.into_iter().enumerate() {
					debug!(
						LOGGER,
						"check_orphans: get block {} at {}{}",
						orphan.block.hash(),
						height,
						if orphans_len > 1 {
							format!(", no.{} of {} orphans", i, orphans_len)
						} else {
							String::new()
						},
					);
					let height = orphan.block.header.height;
					let res = self.process_block_single(orphan.block, orphan.opts);
					if res.is_ok() {
						orphan_accepted = true;
						height_accepted = height;
					}
				}

				if orphan_accepted {
					// We accepted a block, so see if we can accept any orphans
					height = height_accepted + 1;
					continue;
				}
			}
			break;
		}

		if initial_height != height {
			debug!(
				LOGGER,
				"check_orphans: {} blocks accepted since height {}, remaining # orphans {}",
				height - initial_height,
				initial_height,
				self.orphans.len(),
			);
		}
	}

	/// TODO - where do we call this from? And do we need a rewind first?
	/// For the given commitment find the unspent output and return the
	/// associated Return an error if the output does not exist or has been
	/// spent. This querying is done in a way that is consistent with the
	/// current chain state, specifically the current winning (valid, most
	/// work) fork.
	pub fn is_unspent(&self, output_ref: &OutputIdentifier) -> Result<Hash, Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		let res = txhashset.is_unspent(output_ref);
		match res {
			Err(e) => Err(e),
			Ok((h, _)) => Ok(h),
		}
	}

	/// Validate the tx against the current UTXO set.
	pub fn validate_tx(&self, tx: &Transaction) -> Result<(), Error> {
		let txhashset = self.txhashset.read().unwrap();
		txhashset::utxo_view(&txhashset, |utxo| {
			utxo.validate_tx(tx)?;
			Ok(())
		})
	}

	fn next_block_height(&self) -> Result<u64, Error> {
		let bh = self.head_header()?;
		Ok(bh.height + 1)
	}

	/// Verify we are not attempting to spend a coinbase output
	/// that has not yet sufficiently matured.
	pub fn verify_coinbase_maturity(&self, tx: &Transaction) -> Result<(), Error> {
		let height = self.next_block_height()?;
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset::extending_readonly(&mut txhashset, |extension| {
			extension.verify_coinbase_maturity(&tx.inputs(), height)?;
			Ok(())
		})
	}

	/// Verify that the tx has a lock_height that is less than or equal to
	/// the height of the next block.
	pub fn verify_tx_lock_height(&self, tx: &Transaction) -> Result<(), Error> {
		let height = self.next_block_height()?;
		if tx.lock_height() <= height {
			Ok(())
		} else {
			Err(ErrorKind::TxLockHeight.into())
		}
	}

	/// Validate the current chain state.
	pub fn validate(&self, fast_validation: bool) -> Result<(), Error> {
		let header = self.store.head_header()?;

		// Lets just treat an "empty" node that just got started up as valid.
		if header.height == 0 {
			return Ok(());
		}

		let mut txhashset = self.txhashset.write().unwrap();

		// Now create an extension from the txhashset and validate against the
		// latest block header. Rewind the extension to the specified header to
		// ensure the view is consistent.
		txhashset::extending_readonly(&mut txhashset, |extension| {
			extension.rewind(&header)?;
			extension.validate(fast_validation, &NoStatus)?;
			Ok(())
		})
	}

	/// Sets the txhashset roots on a brand new block by applying the block on
	/// the current txhashset state.
	pub fn set_txhashset_roots(&self, b: &mut Block, is_fork: bool) -> Result<(), Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		let (roots, sizes) = txhashset::extending_readonly(&mut txhashset, |extension| {
			if is_fork {
				pipe::rewind_and_apply_fork(b, extension)?;
			}
			extension.apply_block(b)?;
			Ok((extension.roots(), extension.sizes()))
		})?;

		b.header.output_root = roots.output_root;
		b.header.range_proof_root = roots.rproof_root;
		b.header.kernel_root = roots.kernel_root;
		b.header.output_mmr_size = sizes.0;
		b.header.kernel_mmr_size = sizes.2;
		Ok(())
	}

	/// Return a pre-built Merkle proof for the given commitment from the store.
	pub fn get_merkle_proof(
		&self,
		output: &OutputIdentifier,
		block_header: &BlockHeader,
	) -> Result<MerkleProof, Error> {
		let mut txhashset = self.txhashset.write().unwrap();

		let merkle_proof = txhashset::extending_readonly(&mut txhashset, |extension| {
			extension.rewind(&block_header)?;
			extension.merkle_proof(output)
		})?;

		Ok(merkle_proof)
	}

	/// Return a merkle proof valid for the current output pmmr state at the
	/// given pos
	pub fn get_merkle_proof_for_pos(&self, commit: Commitment) -> Result<MerkleProof, String> {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset.merkle_proof(commit)
	}

	/// Returns current txhashset roots
	pub fn get_txhashset_roots(&self) -> (Hash, Hash, Hash) {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset.roots()
	}

	/// Provides a reading view into the current txhashset state as well as
	/// the required indexes for a consumer to rewind to a consistent state
	/// at the provided block hash.
	pub fn txhashset_read(&self, h: Hash) -> Result<(u64, u64, File), Error> {
		// now we want to rewind the txhashset extension and
		// sync a "rewound" copy of the leaf_set files to disk
		// so we can send these across as part of the zip file.
		// The fast sync client does *not* have the necessary data
		// to rewind after receiving the txhashset zip.
		let header = self.get_block_header(&h)?;
		{
			let mut txhashset = self.txhashset.write().unwrap();
			txhashset::extending_readonly(&mut txhashset, |extension| {
				extension.rewind(&header)?;
				extension.snapshot()?;
				Ok(())
			})?;
		}

		// prepares the zip and return the corresponding Read
		let txhashset_reader = txhashset::zip_read(self.db_root.clone(), &header)?;
		Ok((
			header.output_mmr_size,
			header.kernel_mmr_size,
			txhashset_reader,
		))
	}

	// Special handling to make sure the whole kernel set matches each of its
	// roots in each block header, without truncation. We go back header by
	// header, rewind and check each root. This fixes a potential weakness in
	// fast sync where a reorg past the horizon could allow a whole rewrite of
	// the kernel set.
	fn validate_kernel_history(
		&self,
		header: &BlockHeader,
		txhashset: &txhashset::TxHashSet,
	) -> Result<(), Error> {
		debug!(
			LOGGER,
			"chain: validate_kernel_history: rewinding and validating kernel history (readonly)"
		);

		let mut count = 0;
		let mut current = header.clone();
		txhashset::rewindable_kernel_view(&txhashset, |view| {
			while current.height > 0 {
				view.rewind(&current)?;
				view.validate_root()?;
				current = view.batch().get_block_header(&current.previous)?;
				count += 1;
			}
			Ok(())
		})?;

		debug!(
			LOGGER,
			"chain: validate_kernel_history: validated kernel root on {} headers", count,
		);

		Ok(())
	}

	/// Writes a reading view on a txhashset state that's been provided to us.
	/// If we're willing to accept that new state, the data stream will be
	/// read as a zip file, unzipped and the resulting state files should be
	/// rewound to the provided indexes.
	pub fn txhashset_write(
		&self,
		h: Hash,
		txhashset_data: File,
		status: &TxHashsetWriteStatus,
	) -> Result<(), Error> {
		status.on_setup();

		// Initial check based on relative heights of current head and header_head.
		{
			let head = self.head().unwrap();
			let header_head = self.header_head().unwrap();
			if header_head.height - head.height < global::cut_through_horizon() as u64 {
				return Err(ErrorKind::InvalidTxHashSet("not needed".to_owned()).into());
			}
		}

		let header = self.get_block_header(&h)?;
		txhashset::zip_write(self.db_root.clone(), txhashset_data, &header)?;

		let mut txhashset =
			txhashset::TxHashSet::open(self.db_root.clone(), self.store.clone(), Some(&header))?;

		// Validate the full kernel history (kernel MMR root for every block header).
		self.validate_kernel_history(&header, &txhashset)?;

		// all good, prepare a new batch and update all the required records
		debug!(
			LOGGER,
			"chain: txhashset_write: rewinding a 2nd time (writeable)"
		);

		let mut batch = self.store.batch()?;

		txhashset::extending(&mut txhashset, &mut batch, |extension| {
			extension.rewind(&header)?;

			// Validate the extension, generating the utxo_sum and kernel_sum.
			// Full validation, including rangeproofs and kernel signature verification.
			let (utxo_sum, kernel_sum) = extension.validate(false, status)?;

			// Now that we have block_sums the total_kernel_sum on the block_header is redundant.
			if header.total_kernel_sum != kernel_sum {
				return Err(
					ErrorKind::Other(format!("total_kernel_sum in header does not match")).into(),
				);
			}

			// Save the block_sums (utxo_sum, kernel_sum) to the db for use later.
			extension.batch.save_block_sums(
				&header.hash(),
				&BlockSums {
					utxo_sum,
					kernel_sum,
				},
			)?;

			extension.rebuild_index()?;
			Ok(())
		})?;

		debug!(
			LOGGER,
			"chain: txhashset_write: finished validating and rebuilding"
		);

		status.on_save();

		// Replace the chain txhashset with the newly built one.
		{
			let mut txhashset_ref = self.txhashset.write().unwrap();
			*txhashset_ref = txhashset;
		}

		debug!(
			LOGGER,
			"chain: txhashset_write: replaced our txhashset with the new one"
		);

		// Save the new head to the db and rebuild the header by height index.
		{
			let tip = Tip::from_block(&header);
			batch.save_body_head(&tip)?;
			batch.save_header_height(&header)?;
			batch.build_by_height_index(&header, true)?;
		}

		// Commit all the changes to the db.
		batch.commit()?;

		debug!(
			LOGGER,
			"chain: txhashset_write: finished committing the batch (head etc.)"
		);

		// Check for any orphan blocks and process them based on the new chain state.
		self.check_orphans(header.height + 1);

		status.on_done();
		Ok(())
	}

	/// Triggers chain compaction, cleaning up some unnecessary historical
	/// information. We introduce a chain depth called horizon, which is
	/// typically in the range of a couple days. Before that horizon, this
	/// method will:
	///
	/// * compact the MMRs data files and flushing the corresponding remove logs
	/// * delete old records from the k/v store (older blocks, indexes, etc.)
	///
	/// This operation can be resource intensive and takes some time to execute.
	/// Meanwhile, the chain will not be able to accept new blocks. It should
	/// therefore be called judiciously.
	pub fn compact(&self) -> Result<(), Error> {
		if self.archive_mode {
			debug!(
				LOGGER,
				"Blockchain compaction disabled, node running in archive mode."
			);
			return Ok(());
		}

		debug!(LOGGER, "Starting blockchain compaction.");
		// Compact the txhashset via the extension.
		{
			let mut txhashset = self.txhashset.write().unwrap();
			txhashset.compact()?;

			// print out useful debug info after compaction
			txhashset::extending_readonly(&mut txhashset, |extension| {
				extension.dump_output_pmmr();
				Ok(())
			})?;
		}

		// Now check we can still successfully validate the chain state after
		// compacting, shouldn't be necessary once all of this is well-oiled
		debug!(LOGGER, "Validating state after compaction.");
		self.validate(true)?;

		// we need to be careful here in testing as 20 blocks is not that long
		// in wall clock time
		let horizon = global::cut_through_horizon() as u64;
		let head = self.head()?;

		if head.height <= horizon {
			return Ok(());
		}

		debug!(
			LOGGER,
			"Compaction remove blocks older than {}.",
			head.height - horizon
		);
		let mut count = 0;
		let batch = self.store.batch()?;
		let mut current = batch.get_header_by_height(head.height - horizon - 1)?;
		loop {
			// Go to the store directly so we can handle NotFoundErr robustly.
			match self.store.get_block(&current.hash()) {
				Ok(b) => {
					batch.delete_block(&b.hash())?;
					count += 1;
				}
				Err(NotFoundErr(_)) => {
					break;
				}
				Err(e) => {
					return Err(
						ErrorKind::StoreErr(e, "retrieving block to compact".to_owned()).into(),
					)
				}
			}
			if current.height <= 1 {
				break;
			}
			match batch.get_block_header(&current.previous) {
				Ok(h) => current = h,
				Err(NotFoundErr(_)) => break,
				Err(e) => return Err(From::from(e)),
			}
		}
		batch.commit()?;
		debug!(LOGGER, "Compaction removed {} blocks, done.", count);
		Ok(())
	}

	/// returns the last n nodes inserted into the output sum tree
	pub fn get_last_n_output(&self, distance: u64) -> Vec<(Hash, OutputIdentifier)> {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset.last_n_output(distance)
	}

	/// as above, for rangeproofs
	pub fn get_last_n_rangeproof(&self, distance: u64) -> Vec<(Hash, RangeProof)> {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset.last_n_rangeproof(distance)
	}

	/// as above, for kernels
	pub fn get_last_n_kernel(&self, distance: u64) -> Vec<(Hash, TxKernel)> {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset.last_n_kernel(distance)
	}

	/// outputs by insertion index
	pub fn unspent_outputs_by_insertion_index(
		&self,
		start_index: u64,
		max: u64,
	) -> Result<(u64, u64, Vec<Output>), Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		let max_index = txhashset.highest_output_insertion_index();
		let outputs = txhashset.outputs_by_insertion_index(start_index, max);
		let rangeproofs = txhashset.rangeproofs_by_insertion_index(start_index, max);
		if outputs.0 != rangeproofs.0 || outputs.1.len() != rangeproofs.1.len() {
			return Err(ErrorKind::TxHashSetErr(String::from(
				"Output and rangeproof sets don't match",
			)).into());
		}
		let mut output_vec: Vec<Output> = vec![];
		for (ref x, &y) in outputs.1.iter().zip(rangeproofs.1.iter()) {
			output_vec.push(Output {
				commit: x.commit,
				features: x.features,
				proof: y,
			});
		}
		Ok((outputs.0, max_index, output_vec))
	}

	/// Orphans pool size
	pub fn orphans_len(&self) -> usize {
		self.orphans.len()
	}

	/// Reset header_head and sync_head to head of current body chain
	pub fn reset_head(&self) -> Result<(), Error> {
		let batch = self.store.batch()?;
		batch.reset_head()?;
		batch.commit()?;
		Ok(())
	}

	/// Tip (head) of the block chain.
	pub fn head(&self) -> Result<Tip, Error> {
		self.store
			.head()
			.map_err(|e| ErrorKind::StoreErr(e, "chain head".to_owned()).into())
	}

	/// Tip (head) of the header chain.
	pub fn header_head(&self) -> Result<Tip, Error> {
		self.store
			.header_head()
			.map_err(|e| ErrorKind::StoreErr(e, "chain header head".to_owned()).into())
	}

	/// Block header for the chain head
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.store
			.head_header()
			.map_err(|e| ErrorKind::StoreErr(e, "chain head header".to_owned()).into())
	}

	/// Gets a block header by hash
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		self.store
			.get_block(h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get block".to_owned()).into())
	}

	/// Gets a block header by hash
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		self.store
			.get_block_header(h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get header".to_owned()).into())
	}

	/// Get block_sums by header hash.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		self.store
			.get_block_sums(h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get block_sums".to_owned()).into())
	}

	/// Gets the block header at the provided height
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		self.store
			.get_header_by_height(height)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get header by height".to_owned()).into())
	}

	/// Gets the block header in which a given output appears in the txhashset
	pub fn get_header_for_output(
		&self,
		output_ref: &OutputIdentifier,
	) -> Result<BlockHeader, Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		let (_, pos) = txhashset.is_unspent(output_ref)?;
		let mut min = 1;
		let mut max = {
			let h = self.head()?;
			h.height
		};

		loop {
			let search_height = max - (max - min) / 2;
			let h = self.get_header_by_height(search_height)?;
			let h_prev = self.get_header_by_height(search_height - 1)?;
			if pos > h.output_mmr_size {
				min = search_height;
			} else if pos < h_prev.output_mmr_size {
				max = search_height;
			} else {
				if pos == h_prev.output_mmr_size {
					return Ok(h_prev);
				}
				return Ok(h);
			}
		}
	}

	/// Verifies the given block header is actually on the current chain.
	/// Checks the header_by_height index to verify the header is where we say
	/// it is
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		self.store
			.is_on_current_chain(header)
			.map_err(|e| ErrorKind::StoreErr(e, "chain is_on_current_chain".to_owned()).into())
	}

	/// Get the tip of the current "sync" header chain.
	/// This may be significantly different to current header chain.
	pub fn get_sync_head(&self) -> Result<Tip, Error> {
		self.store
			.get_sync_head()
			.map_err(|e| ErrorKind::StoreErr(e, "chain get sync head".to_owned()).into())
	}

	/// Builds an iterator on blocks starting from the current chain head and
	/// running backward. Specialized to return information pertaining to block
	/// difficulty calculation (timestamp and previous difficulties).
	pub fn difficulty_iter(&self) -> store::DifficultyIter {
		let head = self.head().unwrap();
		let batch = self.store.batch().unwrap();
		store::DifficultyIter::from(head.last_block_h, batch)
	}

	/// Check whether we have a block without reading it
	pub fn block_exists(&self, h: Hash) -> Result<bool, Error> {
		self.store
			.block_exists(&h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain block exists".to_owned()).into())
	}

	/// Reset sync_head to the provided head.
	pub fn reset_sync_head(&self, head: &Tip) -> Result<(), Error> {
		let batch = self.store.batch()?;
		batch.save_sync_head(head)?;
		batch.commit()?;
		Ok(())
	}
}

fn setup_head(
	genesis: Block,
	store: Arc<store::ChainStore>,
	txhashset: &mut txhashset::TxHashSet,
) -> Result<(), Error> {
	let mut batch = store.batch()?;

	// check if we have a head in store, otherwise the genesis block is it
	let head_res = batch.head();
	let mut head: Tip;
	match head_res {
		Ok(h) => {
			head = h;
			loop {
				// Use current chain tip if we have one.
				// Note: We are rewinding and validating against a writeable extension.
				// If validation is successful we will truncate the backend files
				// to match the provided block header.
				let header = batch.get_block_header(&head.last_block_h)?;

				let res = txhashset::extending(txhashset, &mut batch, |extension| {
					extension.rewind(&header)?;
					extension.validate_roots()?;

					// now check we have the "block sums" for the block in question
					// if we have no sums (migrating an existing node) we need to go
					// back to the txhashset and sum the outputs and kernels
					if header.height > 0 && extension.batch.get_block_sums(&header.hash()).is_err()
					{
						debug!(
							LOGGER,
							"chain: init: building (missing) block sums for {} @ {}",
							header.height,
							header.hash()
						);

						// Do a full (and slow) validation of the txhashset extension
						// to calculate the utxo_sum and kernel_sum at this block height.
						let (utxo_sum, kernel_sum) = extension.validate_kernel_sums()?;

						// Save the block_sums to the db for use later.
						extension.batch.save_block_sums(
							&header.hash(),
							&BlockSums {
								utxo_sum,
								kernel_sum,
							},
						)?;
					}

					debug!(
						LOGGER,
						"chain: init: rewinding and validating before we start... {} at {}",
						header.hash(),
						header.height,
					);
					Ok(())
				});

				if res.is_ok() {
					break;
				} else {
					// We may have corrupted the MMR backend files last time we stopped the
					// node. If this appears to be the case revert the head to the previous
					// header and try again
					let prev_header = batch.get_block_header(&head.prev_block_h)?;
					let _ = batch.delete_block(&header.hash());
					let _ = batch.setup_height(&prev_header, &head)?;
					head = Tip::from_block(&prev_header);
					batch.save_head(&head)?;
				}
			}
		}
		Err(NotFoundErr(_)) => {
			batch.save_block(&genesis)?;
			let tip = Tip::from_block(&genesis.header);
			batch.save_head(&tip)?;
			batch.setup_height(&genesis.header, &tip)?;

			txhashset::extending(txhashset, &mut batch, |extension| {
				extension.apply_block(&genesis)?;

				// Save the block_sums to the db for use later.
				extension
					.batch
					.save_block_sums(&genesis.hash(), &BlockSums::default())?;

				Ok(())
			})?;

			info!(LOGGER, "chain: init: saved genesis: {:?}", genesis.hash());
		}
		Err(e) => return Err(ErrorKind::StoreErr(e, "chain init load head".to_owned()))?,
	};

	// Initialize header_head and sync_head as necessary for chain init.
	batch.reset_head()?;
	batch.commit()?;

	Ok(())
}
