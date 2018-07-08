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
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use lmdb;

use core::core::hash::{Hash, Hashed};
use core::core::merkle_proof::MerkleProof;
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader, Output, OutputIdentifier, Transaction, TxKernel};
use core::global;
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

#[derive(Debug, Clone)]
struct Orphan {
	block: Block,
	opts: Options,
	added: Instant,
}

struct OrphanBlockPool {
	// blocks indexed by their hash
	orphans: RwLock<HashMap<Hash, Orphan>>,
	// additional index of height -> hash
	// so we can efficiently identify a child block (ex-orphan) after processing a block
	height_idx: RwLock<HashMap<u64, Vec<Hash>>>,
}

impl OrphanBlockPool {
	fn new() -> OrphanBlockPool {
		OrphanBlockPool {
			orphans: RwLock::new(HashMap::new()),
			height_idx: RwLock::new(HashMap::new()),
		}
	}

	fn len(&self) -> usize {
		let orphans = self.orphans.read().unwrap();
		orphans.len()
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

	fn contains(&self, hash: &Hash) -> bool {
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

	head: Arc<Mutex<Tip>>,
	orphans: Arc<OrphanBlockPool>,
	txhashset_lock: Arc<Mutex<bool>>,
	txhashset: Arc<RwLock<txhashset::TxHashSet>>,

	// POW verification function
	pow_verifier: fn(&BlockHeader, u8) -> bool,
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
		pow_verifier: fn(&BlockHeader, u8) -> bool,
	) -> Result<Chain, Error> {
		let chain_store = store::ChainStore::new(db_env)?;

		let store = Arc::new(chain_store);

		// open the txhashset, creating a new one if necessary
		let mut txhashset = txhashset::TxHashSet::open(db_root.clone(), store.clone(), None)?;

		setup_head(genesis, store.clone(), &mut txhashset)?;

		// Now reload the chain head (either existing head or genesis from above)
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
			head: Arc::new(Mutex::new(head)),
			orphans: Arc::new(OrphanBlockPool::new()),
			txhashset_lock: Arc::new(Mutex::new(false)),
			txhashset: Arc::new(RwLock::new(txhashset)),
			pow_verifier: pow_verifier,
		})
	}

	/// Processes a single block, then checks for orphans, processing
	/// those as well if they're found
	pub fn process_block(
		&self,
		b: Block,
		opts: Options,
	) -> Result<(Option<Tip>, Option<Block>), Error> {
		let res = self.process_block_no_orphans(b, opts);
		match res {
			Ok((t, b)) => {
				// We accepted a block, so see if we can accept any orphans
				if let Some(ref b) = b {
					self.check_orphans(b.header.height + 1);
				}
				Ok((t, b))
			}
			Err(e) => Err(e),
		}
	}

	/// Attempt to add a new block to the chain. Returns the new chain tip if it
	/// has been added to the longest chain, None if it's added to an (as of
	/// now) orphan chain.
	pub fn process_block_no_orphans(
		&self,
		b: Block,
		opts: Options,
	) -> Result<(Option<Tip>, Option<Block>), Error> {
		let head = self.store.head()?;
		let mut ctx = self.ctx_from_head(head, opts)?;

		let res = pipe::process_block(&b, &mut ctx);

		match res {
			Ok(Some(ref tip)) => {
				// block got accepted and extended the head, updating our head
				let chain_head = self.head.clone();
				{
					let mut head = chain_head.lock().unwrap();
					*head = tip.clone();
				}

				// notifying other parts of the system of the update
				if !opts.contains(Options::SYNC) {
					// broadcast the block
					let adapter = self.adapter.clone();
					adapter.block_accepted(&b, opts);
				}
				Ok((Some(tip.clone()), Some(b)))
			}
			Ok(None) => {
				// block got accepted but we did not extend the head
				// so its on a fork (or is the start of a new fork)
				// broadcast the block out so everyone knows about the fork
				//
				// TODO - This opens us to an amplification attack on blocks
				// mined at a low difficulty. We should suppress really old blocks
				// or less relevant blocks somehow.
				// We should also probably consider banning nodes that send us really old
				// blocks.
				//
				if !opts.contains(Options::SYNC) {
					// broadcast the block
					let adapter = self.adapter.clone();
					adapter.block_accepted(&b, opts);
				}
				Ok((None, Some(b)))
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

						// In the case of a fork - it is possible to have multiple blocks
						// that are children of a given block.
						// We do not handle this currently for orphans (future enhancement?).
						// We just assume "last one wins" for now.
						&self.orphans.add(orphan);

						debug!(
							LOGGER,
							"process_block: orphan: {:?}, # orphans {}",
							block_hash,
							self.orphans.len(),
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
						Err(ErrorKind::Other(format!("{:?}", e).to_owned()).into())
					}
				}
			}
		}
	}

	/// Process a block header received during "header first" propagation.
	pub fn process_block_header(&self, bh: &BlockHeader, opts: Options) -> Result<(), Error> {
		let header_head = self.get_header_head()?;
		let mut ctx = self.ctx_from_head(header_head, opts)?;
		pipe::process_block_header(bh, &mut ctx)
	}

	/// Attempt to add a new header to the header chain.
	/// This is only ever used during sync and uses sync_head.
	pub fn sync_block_header(&self, bh: &BlockHeader, opts: Options) -> Result<Option<Tip>, Error> {
		let sync_head = self.get_sync_head()?;
		let header_head = self.get_header_head()?;
		let mut sync_ctx = self.ctx_from_head(sync_head, opts)?;
		let mut header_ctx = self.ctx_from_head(header_head, opts)?;
		let mut batch = self.store.batch()?;
		let res = pipe::sync_block_header(bh, &mut sync_ctx, &mut header_ctx, &mut batch);
		if res.is_ok() {
			batch.commit()?;
		}
		res
	}

	fn ctx_from_head<'a>(&self, head: Tip, opts: Options) -> Result<pipe::BlockContext, Error> {
		Ok(pipe::BlockContext {
			opts: opts,
			store: self.store.clone(),
			head: head,
			pow_verifier: self.pow_verifier,
			txhashset: self.txhashset.clone(),
		})
	}

	/// Check if hash is for a known orphan.
	pub fn is_orphan(&self, hash: &Hash) -> bool {
		self.orphans.contains(hash)
	}

	/// Check for orphans, once a block is successfully added
	pub fn check_orphans(&self, mut height: u64) {
		trace!(
			LOGGER,
			"chain: check_orphans at {}, # orphans {}",
			height,
			self.orphans.len(),
		);
		// Is there an orphan in our orphans that we can now process?
		loop {
			if let Some(orphans) = self.orphans.remove_by_height(&height) {
				for orphan in orphans {
					let res = self.process_block_no_orphans(orphan.block, orphan.opts);
					if let Ok((_, Some(b))) = res {
						// We accepted a block, so see if we can accept any orphans
						height = b.header.height + 1;
					} else {
						break;
					}
				}
			} else {
				break;
			}
		}
	}

	/// For the given commitment find the unspent output and return the
	/// associated Return an error if the output does not exist or has been
	/// spent. This querying is done in a way that is consistent with the
	/// current chain state, specifically the current winning (valid, most
	/// work) fork.
	pub fn is_unspent(&self, output_ref: &OutputIdentifier) -> Result<Hash, Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset.is_unspent(output_ref)
	}

	fn next_block_height(&self) -> Result<u64, Error> {
		let bh = self.head_header()?;
		Ok(bh.height + 1)
	}

	/// Validate a vec of "raw" transactions against the current chain state.
	/// Specifying a "pre_tx" if we need to adjust the state, for example when
	/// validating the txs in the stempool we adjust the state based on the
	/// txpool.
	pub fn validate_raw_txs(
		&self,
		txs: Vec<Transaction>,
		pre_tx: Option<Transaction>,
	) -> Result<Vec<Transaction>, Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset::extending_readonly(&mut txhashset, |extension| {
			let valid_txs = extension.validate_raw_txs(txs, pre_tx)?;
			Ok(valid_txs)
		})
	}

	/// Verify we are not attempting to spend a coinbase output
	/// that has not yet sufficiently matured.
	pub fn verify_coinbase_maturity(&self, tx: &Transaction) -> Result<(), Error> {
		let height = self.next_block_height()?;
		let mut txhashset = self.txhashset.write().unwrap();
		txhashset::extending_readonly(&mut txhashset, |extension| {
			extension.verify_coinbase_maturity(&tx.inputs, height)?;
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
	pub fn validate(&self, skip_rproofs: bool) -> Result<(), Error> {
		let header = self.store.head_header()?;

		// Lets just treat an "empty" node that just got started up as valid.
		if header.height == 0 {
			return Ok(());
		}

		// We want to validate the full kernel history here for completeness.
		let skip_kernel_hist = false;

		let mut txhashset = self.txhashset.write().unwrap();

		// Now create an extension from the txhashset and validate against the
		// latest block header. Rewind the extension to the specified header to
		// ensure the view is consistent.
		txhashset::extending_readonly(&mut txhashset, |extension| {
			extension.rewind(&header, &header)?;
			extension.validate(&header, skip_rproofs, skip_kernel_hist, &NoStatus)?;
			Ok(())
		})
	}

	/// Sets the txhashset roots on a brand new block by applying the block on
	/// the current txhashset state.
	pub fn set_txhashset_roots(&self, b: &mut Block, is_fork: bool) -> Result<(), Error> {
		let mut txhashset = self.txhashset.write().unwrap();
		let store = self.store.clone();

		let (roots, sizes) = txhashset::extending_readonly(&mut txhashset, |extension| {
			if is_fork {
				pipe::rewind_and_apply_fork(b, store, extension)?;
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
			extension.merkle_proof(output, block_header)
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
		let header = self.store.get_block_header(&h)?;
		let head_header = self.store.head_header()?;
		{
			let mut txhashset = self.txhashset.write().unwrap();
			txhashset::extending_readonly(&mut txhashset, |extension| {
				extension.rewind(&header, &head_header)?;
				extension.snapshot(&header)?;
				Ok(())
			})?;
		}

		// prepares the zip and return the corresponding Read
		let txhashset_reader = txhashset::zip_read(self.db_root.clone())?;
		Ok((
			header.output_mmr_size,
			header.kernel_mmr_size,
			txhashset_reader,
		))
	}

	/// Writes a reading view on a txhashset state that's been provided to us.
	/// If we're willing to accept that new state, the data stream will be
	/// read as a zip file, unzipped and the resulting state files should be
	/// rewound to the provided indexes.
	pub fn txhashset_write<T>(
		&self,
		h: Hash,
		txhashset_data: File,
		status: &T,
	) -> Result<(), Error>
	where
		T: TxHashsetWriteStatus,
	{
		let _ = self.txhashset_lock.lock().unwrap();
		status.on_setup();
		let head = self.head().unwrap();
		let header_head = self.get_header_head().unwrap();
		if header_head.height - head.height < global::cut_through_horizon() as u64 {
			return Err(ErrorKind::InvalidTxHashSet("not needed".to_owned()).into());
		}

		let header = self.store.get_block_header(&h)?;
		txhashset::zip_write(self.db_root.clone(), txhashset_data)?;

		let mut txhashset =
			txhashset::TxHashSet::open(self.db_root.clone(), self.store.clone(), Some(&header))?;

		// validate against a read-only extension first (some of the validation
		// runs additional rewinds)
		debug!(LOGGER, "chain: txhashset_write: rewinding and validating (read-only)");
		txhashset::extending_readonly(&mut txhashset, |extension| {
			extension.rewind(&header, &header)?;
			extension.validate(&header, false, false, status)?;
			Ok(())
		})?;

		// all good, prepare a new batch and update all the required records
		debug!(LOGGER, "chain: txhashset_write: rewinding and validating a 2nd time (writeable)");
		let mut batch = self.store.batch()?;
		txhashset::extending(&mut txhashset, &mut batch, |extension| {
			extension.rewind(&header, &header)?;
			extension.validate(&header, false, true, status)?;
			extension.rebuild_index()?;
			Ok(())
		})?;

		debug!(LOGGER, "chain: txhashset_write: finished validating and rebuilding");

		status.on_save();
		// replace the chain txhashset with the newly built one
		{
			let mut txhashset_ref = self.txhashset.write().unwrap();
			*txhashset_ref = txhashset;
		}
		// setup new head
		{
			let mut head = self.head.lock().unwrap();
			*head = Tip::from_block(&header);
			batch.save_body_head(&head)?;
			batch.save_header_height(&header)?;
			batch.build_by_height_index(&header, true)?;
		}
		batch.commit()?;

		debug!(LOGGER, "chain: txhashset_write: finished committing the batch (head etc.)");

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
		debug!(LOGGER, "Starting blockchain compaction.");
		// Compact the txhashset via the extension.
		{
			let mut txhashes = self.txhashset.write().unwrap();
			txhashes.compact()?;

			// print out useful debug info after compaction
			txhashset::extending_readonly(&mut txhashes, |extension| {
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
		let mut current = self.store.get_header_by_height(head.height - horizon - 1)?;
		let batch = self.store.batch()?;
		loop {
			match self.store.get_block(&current.hash()) {
				Ok(b) => {
					count += 1;
					batch.delete_block(&b.hash())?;
					batch.delete_block_input_bitmap(&b.hash())?;
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
			match self.store.get_block_header(&current.previous) {
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

	/// Total difficulty at the head of the chain
	pub fn total_difficulty(&self) -> Difficulty {
		self.head.lock().unwrap().clone().total_difficulty
	}

	/// Orphans pool size
	pub fn orphans_len(&self) -> usize {
		self.orphans.len()
	}

	/// Total difficulty at the head of the header chain
	pub fn total_header_difficulty(&self) -> Result<Difficulty, Error> {
		Ok(self.store.get_header_head()?.total_difficulty)
	}

	/// Reset header_head and sync_head to head of current body chain
	pub fn reset_head(&self) -> Result<(), Error> {
		let batch = self.store.batch()?;
		batch.reset_head()?;
		batch.commit()?;
		Ok(())
	}

	/// Get the tip that's also the head of the chain
	pub fn head(&self) -> Result<Tip, Error> {
		Ok(self.head.lock().unwrap().clone())
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

	/// Gets the block header at the provided height
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		self.store
			.get_header_by_height(height)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get header by height".to_owned()).into())
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

	/// Get the tip of the header chain.
	pub fn get_header_head(&self) -> Result<Tip, Error> {
		self.store
			.get_header_head()
			.map_err(|e| ErrorKind::StoreErr(e, "chain get header head".to_owned()).into())
	}

	/// Builds an iterator on blocks starting from the current chain head and
	/// running backward. Specialized to return information pertaining to block
	/// difficulty calculation (timestamp and previous difficulties).
	pub fn difficulty_iter(&self) -> store::DifficultyIter {
		let head = self.head.lock().unwrap();
		store::DifficultyIter::from(head.last_block_h, self.store.clone())
	}

	/// Check whether we have a block without reading it
	pub fn block_exists(&self, h: Hash) -> Result<bool, Error> {
		self.store
			.block_exists(&h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain block exists".to_owned()).into())
	}
}

fn setup_head(
	genesis: Block,
	store: Arc<store::ChainStore>,
	txhashset: &mut txhashset::TxHashSet,
) -> Result<(), Error> {
	// check if we have a head in store, otherwise the genesis block is it
	let head_res = store.head();
	let mut batch = store.batch()?;
	let mut head: Tip;
	match head_res {
		Ok(h) => {
			head = h;
			let head_header = store.head_header()?;
			loop {
				// Use current chain tip if we have one.
				// Note: We are rewinding and validating against a writeable extension.
				// If validation is successful we will truncate the backend files
				// to match the provided block header.
				let header = store.get_block_header(&head.last_block_h)?;

				let res = txhashset::extending(txhashset, &mut batch, |extension| {
					extension.rewind(&header, &head_header)?;
					extension.validate_roots(&header)?;
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
					let prev_header = store.get_block_header(&head.prev_block_h)?;
					let _ = batch.delete_block(&header.hash());
					let _ = batch.setup_height(&prev_header, &head)?;
					head = Tip::from_block(&prev_header);
					batch.save_head(&head)?;
				}
			}
		}
		Err(NotFoundErr(_)) => {
			let tip = Tip::from_block(&genesis.header);
			batch.save_block(&genesis)?;
			batch.setup_height(&genesis.header, &tip)?;
			txhashset::extending(txhashset, &mut batch, |extension| {
				extension.apply_block(&genesis)?;
				Ok(())
			})?;

			// saving a new tip based on genesis
			batch.save_head(&tip)?;
			head = tip;
			info!(LOGGER, "chain: init: saved genesis: {:?}", genesis.hash());
		}
		Err(e) => return Err(ErrorKind::StoreErr(e, "chain init load head".to_owned()))?,
	};

	// Initialize header_head and sync_head as necessary for chain init.
	batch.init_sync_head(&head)?;
	batch.commit()?;

	Ok(())
}
