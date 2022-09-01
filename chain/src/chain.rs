// Copyright 2021 The Grin Developers
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

use crate::core::core::merkle_proof::MerkleProof;
use crate::core::core::{
	Block, BlockHeader, BlockSums, Committed, Inputs, KernelFeatures, Output, OutputIdentifier,
	Transaction, TxKernel,
};
use crate::core::global;
use crate::core::pow;
use crate::core::ser::ProtocolVersion;
use crate::error::Error;
use crate::pipe;
use crate::store;
use crate::txhashset;
use crate::txhashset::{Desegmenter, PMMRHandle, Segmenter, TxHashSet};
use crate::types::{
	BlockStatus, ChainAdapter, CommitPos, NoStatus, Options, Tip, TxHashsetWriteStatus,
};
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::RwLock;
use crate::{
	core::core::hash::{Hash, Hashed},
	store::Batch,
	txhashset::{ExtensionPair, HeaderExtension},
};
use grin_store::Error::NotFoundErr;
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
		let orphans = self.orphans.read();
		orphans.len()
	}

	fn len_evicted(&self) -> usize {
		self.evicted.load(Ordering::Relaxed)
	}

	fn add(&self, orphan: Orphan) {
		let mut orphans = self.orphans.write();
		let mut height_idx = self.height_idx.write();
		{
			let height_hashes = height_idx
				.entry(orphan.block.header.height)
				.or_insert_with(|| vec![]);
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
	fn remove_by_height(&self, height: u64) -> Option<Vec<Orphan>> {
		let mut orphans = self.orphans.write();
		let mut height_idx = self.height_idx.write();
		height_idx
			.remove(&height)
			.map(|hs| hs.iter().filter_map(|h| orphans.remove(h)).collect())
	}

	pub fn contains(&self, hash: &Hash) -> bool {
		let orphans = self.orphans.read();
		orphans.contains_key(hash)
	}
}

/// Facade to the blockchain block processing pipeline and storage. Provides
/// the current view of the TxHashSet according to the chain state. Also
/// maintains locking for the pipeline to avoid conflicting processing.
pub struct Chain {
	db_root: String,
	store: Arc<store::ChainStore>,
	adapter: Arc<dyn ChainAdapter + Send + Sync>,
	orphans: Arc<OrphanBlockPool>,
	txhashset: Arc<RwLock<txhashset::TxHashSet>>,
	header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
	pibd_segmenter: Arc<RwLock<Option<Segmenter>>>,
	pibd_desegmenter: Arc<RwLock<Option<Desegmenter>>>,
	// POW verification function
	pow_verifier: fn(&BlockHeader) -> Result<(), pow::Error>,
	denylist: Arc<RwLock<Vec<Hash>>>,
	archive_mode: bool,
	genesis: BlockHeader,
}

impl Chain {
	/// Initializes the blockchain and returns a new Chain instance. Does a
	/// check on the current chain head to make sure it exists and creates one
	/// based on the genesis block if necessary.
	pub fn init(
		db_root: String,
		adapter: Arc<dyn ChainAdapter + Send + Sync>,
		genesis: Block,
		pow_verifier: fn(&BlockHeader) -> Result<(), pow::Error>,
		archive_mode: bool,
	) -> Result<Chain, Error> {
		let store = Arc::new(store::ChainStore::new(&db_root)?);

		// open the txhashset, creating a new one if necessary
		let mut txhashset = txhashset::TxHashSet::open(db_root.clone(), store.clone(), None)?;

		let mut header_pmmr = PMMRHandle::new(
			Path::new(&db_root).join("header").join("header_head"),
			false,
			ProtocolVersion(1),
			None,
		)?;

		setup_head(&genesis, &store, &mut header_pmmr, &mut txhashset)?;

		// Initialize the output_pos index based on UTXO set
		// and NRD kernel_pos index based recent kernel history.
		{
			let batch = store.batch()?;
			txhashset.init_output_pos_index(&header_pmmr, &batch)?;
			txhashset.init_recent_kernel_pos_index(&header_pmmr, &batch)?;
			batch.commit()?;
		}

		let chain = Chain {
			db_root,
			store,
			adapter,
			orphans: Arc::new(OrphanBlockPool::new()),
			txhashset: Arc::new(RwLock::new(txhashset)),
			header_pmmr: Arc::new(RwLock::new(header_pmmr)),
			pibd_segmenter: Arc::new(RwLock::new(None)),
			pibd_desegmenter: Arc::new(RwLock::new(None)),
			pow_verifier,
			denylist: Arc::new(RwLock::new(vec![])),
			archive_mode,
			genesis: genesis.header,
		};

		chain.log_heads()?;

		Ok(chain)
	}

	/// Add provided header hash to our "denylist".
	/// The header corresponding to any "denied" hash will be rejected
	/// and the peer subsequently banned.
	pub fn invalidate_header(&self, hash: Hash) -> Result<(), Error> {
		self.denylist.write().push(hash);
		Ok(())
	}

	/// Reset both head and header_head to the provided header.
	/// Handles simple rewind and more complex fork scenarios.
	/// Used by the reset_chain_head owner api endpoint.
	/// Caller can choose not to rewind headers, which can be used
	/// during PIBD scenarios where it's desirable to restart the PIBD process
	/// without re-downloading the header chain
	pub fn reset_chain_head<T: Into<Tip>>(
		&self,
		head: T,
		rewind_headers: bool,
	) -> Result<(), Error> {
		let head = head.into();

		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;

		let header = batch.get_block_header(&head.hash())?;

		// Rewind and reapply blocks to reset the output/rangeproof/kernel MMR.
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, batch| {
				self.rewind_and_apply_fork(&header, ext, batch)?;
				batch.save_body_head(&head)?;
				Ok(())
			},
		)?;

		if rewind_headers {
			// If the rewind of full blocks was successful then we can rewind the header MMR.
			// Rewind and reapply headers to reset the header MMR.
			txhashset::header_extending(&mut header_pmmr, &mut batch, |ext, batch| {
				self.rewind_and_apply_header_fork(&header, ext, batch)?;
				batch.save_header_head(&head)?;
				Ok(())
			})?;
		}

		batch.commit()?;

		Ok(())
	}

	/// Reset prune lists (when PIBD resets and rolls back the
	/// entire chain, the prune list needs to be manually wiped
	/// as it's currently not included as part of rewind)
	pub fn reset_prune_lists(&self) -> Result<(), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;

		txhashset::extending(&mut header_pmmr, &mut txhashset, &mut batch, |ext, _| {
			let extension = &mut ext.extension;
			extension.reset_prune_lists();
			Ok(())
		})?;
		Ok(())
	}

	/// Reset PIBD head
	pub fn reset_pibd_head(&self) -> Result<(), Error> {
		let batch = self.store.batch()?;
		batch.save_pibd_head(&self.genesis().into())?;
		Ok(())
	}

	/// Are we running with archive_mode enabled?
	pub fn archive_mode(&self) -> bool {
		self.archive_mode
	}

	/// Return our shared header MMR handle.
	pub fn header_pmmr(&self) -> Arc<RwLock<PMMRHandle<BlockHeader>>> {
		self.header_pmmr.clone()
	}

	/// Return our shared txhashset instance.
	pub fn txhashset(&self) -> Arc<RwLock<TxHashSet>> {
		self.txhashset.clone()
	}

	/// return genesis header
	pub fn genesis(&self) -> BlockHeader {
		self.genesis.clone()
	}

	/// Shared store instance.
	pub fn store(&self) -> Arc<store::ChainStore> {
		self.store.clone()
	}

	fn log_heads(&self) -> Result<(), Error> {
		let log_head = |name, head: Tip| {
			debug!(
				"{}: {} @ {} [{}]",
				name,
				head.total_difficulty.to_num(),
				head.height,
				head.hash(),
			);
		};
		log_head("head", self.head()?);
		log_head("header_head", self.header_head()?);
		Ok(())
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

	fn determine_status(
		&self,
		head: Option<Tip>,
		prev: Tip,
		prev_head: Tip,
		fork_point: Tip,
	) -> BlockStatus {
		// If head is updated then we are either "next" block or we just experienced a "reorg" to new head.
		// Otherwise this is a "fork" off the main chain.
		if let Some(head) = head {
			if self.is_on_current_chain(prev_head, head).is_ok() {
				BlockStatus::Next { prev }
			} else {
				BlockStatus::Reorg {
					prev,
					prev_head,
					fork_point,
				}
			}
		} else {
			BlockStatus::Fork {
				prev,
				head: prev_head,
				fork_point,
			}
		}
	}

	/// Quick check for "known" duplicate block up to and including current chain head.
	/// Returns an error if this block is "known".
	pub fn is_known(&self, header: &BlockHeader) -> Result<(), Error> {
		let head = self.head()?;
		if head.hash() == header.hash() {
			return Err(Error::Unfit("duplicate block".into()));
		}
		if header.total_difficulty() <= head.total_difficulty {
			if self.block_exists(header.hash())? {
				return Err(Error::Unfit("duplicate block".into()));
			}
		}
		Ok(())
	}

	// Check if the provided block is an orphan.
	// If block is an orphan add it to our orphan block pool for deferred processing.
	// If this is the "next" block immediately following current head then not an orphan.
	// Or if we have the previous full block then not an orphan.
	fn check_orphan(&self, block: &Block, opts: Options) -> Result<(), Error> {
		let head = self.head()?;
		let is_next = block.header.prev_hash == head.last_block_h;
		if is_next || self.block_exists(block.header.prev_hash)? {
			return Ok(());
		}

		let block_hash = block.hash();
		let orphan = Orphan {
			block: block.clone(),
			opts,
			added: Instant::now(),
		};
		self.orphans.add(orphan);

		debug!(
			"is_orphan: {:?}, # orphans {}{}",
			block_hash,
			self.orphans.len(),
			if self.orphans.len_evicted() > 0 {
				format!(", # evicted {}", self.orphans.len_evicted())
			} else {
				String::new()
			},
		);

		Err(Error::Orphan)
	}

	/// Attempt to add a new block to the chain.
	/// Returns true if it has been added to the longest chain
	/// or false if it has added to a fork (or orphan?).
	fn process_block_single(&self, b: Block, opts: Options) -> Result<Option<Tip>, Error> {
		// Process the header first.
		// If invalid then fail early.
		// If valid then continue with block processing with header_head committed to db etc.
		self.process_block_header(&b.header, opts)?;

		// Check if we already know about this full block.
		self.is_known(&b.header)?;

		// Check if this block is an orphan.
		// Only do this once we know the header PoW is valid.
		self.check_orphan(&b, opts)?;

		let (head, fork_point, prev_head) = {
			let mut header_pmmr = self.header_pmmr.write();
			let mut txhashset = self.txhashset.write();
			let batch = self.store.batch()?;
			let prev_head = batch.head()?;
			let mut ctx = self.new_ctx(opts, batch, &mut header_pmmr, &mut txhashset)?;

			let (head, fork_point) = pipe::process_block(&b, &mut ctx)?;

			ctx.batch.commit()?;

			// release the lock and let the batch go before post-processing
			(head, fork_point, prev_head)
		};

		let prev = self.get_previous_header(&b.header)?;
		let status = self.determine_status(
			head,
			Tip::from_header(&prev),
			prev_head,
			Tip::from_header(&fork_point),
		);

		// notifying other parts of the system of the update
		self.adapter.block_accepted(&b, status, opts);

		Ok(head)
	}

	/// Process a block header received during "header first" propagation.
	/// Note: This will update header MMR and corresponding header_head
	/// if total work increases (on the header chain).
	pub fn process_block_header(&self, bh: &BlockHeader, opts: Options) -> Result<(), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let batch = self.store.batch()?;
		let mut ctx = self.new_ctx(opts, batch, &mut header_pmmr, &mut txhashset)?;
		pipe::process_block_header(bh, &mut ctx)?;
		ctx.batch.commit()?;
		Ok(())
	}

	/// Attempt to add new headers to the header chain (or fork).
	/// This is only ever used during sync and is based on sync_head.
	/// We update header_head here if our total work increases.
	/// Returns the new sync_head (may temporarily diverge from header_head when syncing a long fork).
	pub fn sync_block_headers(
		&self,
		headers: &[BlockHeader],
		sync_head: Tip,
		opts: Options,
	) -> Result<Option<Tip>, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let batch = self.store.batch()?;

		// Sync the chunk of block headers, updating header_head if total work increases.
		let mut ctx = self.new_ctx(opts, batch, &mut header_pmmr, &mut txhashset)?;
		let sync_head = pipe::process_block_headers(headers, sync_head, &mut ctx)?;
		ctx.batch.commit()?;

		Ok(sync_head)
	}

	/// Build a new block processing context.
	pub fn new_ctx<'a>(
		&self,
		opts: Options,
		batch: store::Batch<'a>,
		header_pmmr: &'a mut txhashset::PMMRHandle<BlockHeader>,
		txhashset: &'a mut txhashset::TxHashSet,
	) -> Result<pipe::BlockContext<'a>, Error> {
		let denylist = self.denylist.read().clone();
		Ok(pipe::BlockContext {
			opts,
			pow_verifier: self.pow_verifier,
			header_allowed: Box::new(move |header| {
				pipe::validate_header_denylist(header, &denylist)
			}),
			header_pmmr,
			txhashset,
			batch,
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
	fn check_orphans(&self, mut height: u64) {
		let initial_height = height;

		// Is there an orphan in our orphans that we can now process?
		loop {
			trace!(
				"check_orphans: at {}, # orphans {}",
				height,
				self.orphans.len(),
			);

			let mut orphan_accepted = false;
			let mut height_accepted = height;

			if let Some(orphans) = self.orphans.remove_by_height(height) {
				let orphans_len = orphans.len();
				for (i, orphan) in orphans.into_iter().enumerate() {
					debug!(
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
				"check_orphans: {} blocks accepted since height {}, remaining # orphans {}",
				height - initial_height,
				initial_height,
				self.orphans.len(),
			);
		}
	}

	/// Returns Ok(Some((out, pos))) if output is unspent.
	/// Returns Ok(None) if output is spent.
	/// Returns Err if something went wrong beyond not finding the output.
	pub fn get_unspent(
		&self,
		commit: Commitment,
	) -> Result<Option<(OutputIdentifier, CommitPos)>, Error> {
		self.txhashset.read().get_unspent(commit)
	}

	/// Retrieves an unspent output using its PMMR position
	pub fn get_unspent_output_at(&self, pos0: u64) -> Result<Output, Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		txhashset::utxo_view(&header_pmmr, &txhashset, |utxo, _| {
			utxo.get_unspent_output_at(pos0)
		})
	}

	/// Validate the tx against the current UTXO set and recent kernels (NRD relative lock heights).
	pub fn validate_tx(&self, tx: &Transaction) -> Result<(), Error> {
		self.validate_tx_against_utxo(tx)?;
		self.validate_tx_kernels(tx)?;
		Ok(())
	}

	/// Validates NRD relative height locks against "recent" kernel history.
	/// Applies the kernels to the current kernel MMR in a readonly extension.
	/// The extension and the db batch are discarded.
	/// The batch ensures duplicate NRD kernels within the tx are handled correctly.
	fn validate_tx_kernels(&self, tx: &Transaction) -> Result<(), Error> {
		let has_nrd_kernel = tx.kernels().iter().any(|k| match k.features {
			KernelFeatures::NoRecentDuplicate { .. } => true,
			_ => false,
		});
		if !has_nrd_kernel {
			return Ok(());
		}
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let height = self.next_block_height()?;
			ext.extension.apply_kernels(tx.kernels(), height, batch)
		})
	}

	fn validate_tx_against_utxo(
		&self,
		tx: &Transaction,
	) -> Result<Vec<(OutputIdentifier, CommitPos)>, Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		txhashset::utxo_view(&header_pmmr, &txhashset, |utxo, batch| {
			utxo.validate_tx(tx, batch)
		})
	}

	/// Validates inputs against the current utxo.
	/// Each input must spend an unspent output.
	/// Returns the vec of output identifiers and their pos of the outputs
	/// that would be spent by the inputs.
	pub fn validate_inputs(
		&self,
		inputs: &Inputs,
	) -> Result<Vec<(OutputIdentifier, CommitPos)>, Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		txhashset::utxo_view(&header_pmmr, &txhashset, |utxo, batch| {
			utxo.validate_inputs(inputs, batch)
		})
	}

	fn next_block_height(&self) -> Result<u64, Error> {
		let bh = self.head_header()?;
		Ok(bh.height + 1)
	}

	/// Verify we are not attempting to spend a coinbase output
	/// that has not yet sufficiently matured.
	pub fn verify_coinbase_maturity(&self, inputs: &Inputs) -> Result<(), Error> {
		let height = self.next_block_height()?;
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		txhashset::utxo_view(&header_pmmr, &txhashset, |utxo, batch| {
			utxo.verify_coinbase_maturity(inputs, height, batch)?;
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
			Err(Error::TxLockHeight)
		}
	}

	/// Validate the current chain state.
	pub fn validate(&self, fast_validation: bool) -> Result<(), Error> {
		let header = self.store.head_header()?;

		// Lets just treat an "empty" node that just got started up as valid.
		if header.height == 0 {
			return Ok(());
		}

		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		// Now create an extension from the txhashset and validate against the
		// latest block header. Rewind the extension to the specified header to
		// ensure the view is consistent.
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			self.rewind_and_apply_fork(&header, ext, batch)?;
			ext.extension.validate(
				&self.genesis,
				fast_validation,
				&NoStatus,
				None,
				None,
				&header,
				None,
			)?;
			Ok(())
		})
	}

	/// Sets prev_root on a brand new block header by applying the previous header to the header MMR.
	pub fn set_prev_root_only(&self, header: &mut BlockHeader) -> Result<(), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let prev_root =
			txhashset::header_extending_readonly(&mut header_pmmr, &self.store(), |ext, batch| {
				let prev_header = batch.get_previous_header(header)?;
				self.rewind_and_apply_header_fork(&prev_header, ext, batch)?;
				ext.root()
			})?;

		// Set the prev_root on the header.
		header.prev_root = prev_root;

		Ok(())
	}

	/// Sets the txhashset roots on a brand new block by applying the block on
	/// the current txhashset state.
	pub fn set_txhashset_roots(&self, b: &mut Block) -> Result<(), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		let (prev_root, roots, sizes) =
			txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
				let previous_header = batch.get_previous_header(&b.header)?;
				self.rewind_and_apply_fork(&previous_header, ext, batch)?;

				let extension = &mut ext.extension;
				let header_extension = &mut ext.header_extension;

				// Retrieve the header root before we apply the new block
				let prev_root = header_extension.root()?;

				// Apply the latest block to the chain state via the extension.
				extension.apply_block(b, header_extension, batch)?;

				Ok((prev_root, extension.roots()?, extension.sizes()))
			})?;

		// Set the output and kernel MMR sizes.
		// Note: We need to do this *before* calculating the roots as the output_root
		// depends on the output_mmr_size
		{
			// Carefully destructure these correctly...
			let (output_mmr_size, _, kernel_mmr_size) = sizes;
			b.header.output_mmr_size = output_mmr_size;
			b.header.kernel_mmr_size = kernel_mmr_size;
		}

		// Set the prev_root on the header.
		b.header.prev_root = prev_root;

		// Set the output, rangeproof and kernel MMR roots.
		b.header.output_root = roots.output_root(&b.header);
		b.header.range_proof_root = roots.rproof_root;
		b.header.kernel_root = roots.kernel_root;

		Ok(())
	}

	/// Return a Merkle proof for the given commitment from the store.
	pub fn get_merkle_proof<T: AsRef<OutputIdentifier>>(
		&self,
		out_id: T,
		header: &BlockHeader,
	) -> Result<MerkleProof, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let merkle_proof =
			txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
				self.rewind_and_apply_fork(&header, ext, batch)?;
				ext.extension.merkle_proof(out_id, batch)
			})?;

		Ok(merkle_proof)
	}

	/// Return a merkle proof valid for the current output pmmr state at the
	/// given pos
	pub fn get_merkle_proof_for_pos(&self, commit: Commitment) -> Result<MerkleProof, Error> {
		let mut txhashset = self.txhashset.write();
		txhashset.merkle_proof(commit)
	}

	/// Rewind and apply fork with the chain specific header validation (denylist) rules.
	/// If we rewind and re-apply a "denied" block then validation will fail.
	fn rewind_and_apply_fork(
		&self,
		header: &BlockHeader,
		ext: &mut ExtensionPair,
		batch: &Batch,
	) -> Result<BlockHeader, Error> {
		let denylist = self.denylist.read().clone();
		pipe::rewind_and_apply_fork(header, ext, batch, &|header| {
			pipe::validate_header_denylist(header, &denylist)
		})
	}

	/// Rewind and apply fork with the chain specific header validation (denylist) rules.
	/// If we rewind and re-apply a "denied" header then validation will fail.
	fn rewind_and_apply_header_fork(
		&self,
		header: &BlockHeader,
		ext: &mut HeaderExtension,
		batch: &Batch,
	) -> Result<(), Error> {
		let denylist = self.denylist.read().clone();
		pipe::rewind_and_apply_header_fork(header, ext, batch, &|header| {
			pipe::validate_header_denylist(header, &denylist)
		})
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

		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			self.rewind_and_apply_fork(&header, ext, batch)?;
			ext.extension.snapshot(batch)?;

			// prepare the zip
			txhashset::zip_read(self.db_root.clone(), &header)
				.map(|file| (header.output_mmr_size, header.kernel_mmr_size, file))
		})
	}

	/// The segmenter is responsible for generation PIBD segments.
	/// We cache a segmenter instance based on the current archve period (new period every 12 hours).
	/// This allows us to efficiently generate bitmap segments for the current archive period.
	///
	/// It is a relatively expensive operation to initializa and cache a new segmenter instance
	/// as this involves rewinding the txhashet by approx 720 blocks (12 hours).
	///
	/// Caller is responsible for only doing this when required.
	/// Caller should verify a peer segment request is valid before calling this for example.
	///
	pub fn segmenter(&self) -> Result<Segmenter, Error> {
		// The archive header corresponds to the data we will segment.
		let ref archive_header = self.txhashset_archive_header()?;

		// Use our cached segmenter if we have one and the associated header matches.
		if let Some(x) = self.pibd_segmenter.read().as_ref() {
			if x.header() == archive_header {
				return Ok(x.clone());
			}
		}

		// We have no cached segmenter or the cached segmenter is no longer useful.
		// Initialize a new segment, cache it and return it.
		let segmenter = self.init_segmenter(archive_header)?;
		let mut cache = self.pibd_segmenter.write();
		*cache = Some(segmenter.clone());

		return Ok(segmenter);
	}

	/// This is an expensive rewind to recreate bitmap state but we only need to do this once.
	/// Caller is responsible for "caching" the segmenter (per archive period) for reuse.
	fn init_segmenter(&self, header: &BlockHeader) -> Result<Segmenter, Error> {
		let now = Instant::now();
		debug!(
			"init_segmenter: initializing new segmenter for {} at {}",
			header.hash(),
			header.height
		);

		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		let bitmap_snapshot =
			txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
				ext.extension.rewind(header, batch)?;
				Ok(ext.extension.bitmap_accumulator())
			})?;

		debug!("init_segmenter: done, took {}ms", now.elapsed().as_millis());

		Ok(Segmenter::new(
			self.txhashset(),
			Arc::new(bitmap_snapshot),
			header.clone(),
		))
	}

	/// instantiate desegmenter (in same lazy fashion as segmenter, though this should not be as
	/// expensive an operation)
	pub fn desegmenter(
		&self,
		archive_header: &BlockHeader,
	) -> Result<Arc<RwLock<Option<Desegmenter>>>, Error> {
		// Use our cached desegmenter if we have one and the associated header matches.
		if let Some(d) = self.pibd_desegmenter.write().as_ref() {
			if d.header() == archive_header {
				return Ok(self.pibd_desegmenter.clone());
			}
		}

		let desegmenter = self.init_desegmenter(archive_header)?;
		let mut cache = self.pibd_desegmenter.write();
		*cache = Some(desegmenter.clone());

		Ok(self.pibd_desegmenter.clone())
	}

	/// initialize a desegmenter, which is capable of extending the hashset by appending
	/// PIBD segments of the three PMMR trees + Bitmap PMMR
	/// header should be the same header as selected for the txhashset.zip archive
	fn init_desegmenter(&self, header: &BlockHeader) -> Result<Desegmenter, Error> {
		debug!(
			"init_desegmenter: initializing new desegmenter for {} at {}",
			header.hash(),
			header.height
		);

		Ok(Desegmenter::new(
			self.txhashset(),
			self.header_pmmr.clone(),
			header.clone(),
			self.genesis.clone(),
			self.store.clone(),
		))
	}

	/// To support the ability to download the txhashset from multiple peers in parallel,
	/// the peers must all agree on the exact binary representation of the txhashset.
	/// This means compacting and rewinding to the exact same header.
	/// Since compaction is a heavy operation, peers can agree to compact every 12 hours,
	/// and no longer support requesting arbitrary txhashsets.
	/// Here we return the header of the txhashset we are currently offering to peers.
	pub fn txhashset_archive_header(&self) -> Result<BlockHeader, Error> {
		let sync_threshold = global::state_sync_threshold() as u64;
		let body_head = self.head()?;
		let archive_interval = global::txhashset_archive_interval();
		let mut txhashset_height = body_head.height.saturating_sub(sync_threshold);
		txhashset_height = txhashset_height.saturating_sub(txhashset_height % archive_interval);

		debug!(
			"txhashset_archive_header: body_head - {}, {}, txhashset height - {}",
			body_head.last_block_h, body_head.height, txhashset_height,
		);

		self.get_header_by_height(txhashset_height)
	}

	/// Return the Block Header at the txhashset horizon, considering only the
	/// contents of the header PMMR
	pub fn txhashset_archive_header_header_only(&self) -> Result<BlockHeader, Error> {
		let header_head = self.header_head()?;
		let threshold = global::state_sync_threshold() as u64;
		let archive_interval = global::txhashset_archive_interval();
		let mut txhashset_height = header_head.height.saturating_sub(threshold);
		txhashset_height = txhashset_height.saturating_sub(txhashset_height % archive_interval);
		self.get_header_by_height(txhashset_height)
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
		debug!("validate_kernel_history: rewinding and validating kernel history (readonly)");

		let mut count = 0;
		let mut current = header.clone();
		txhashset::rewindable_kernel_view(&txhashset, |view, batch| {
			while current.height > 0 {
				view.rewind(&current)?;
				view.validate_root()?;
				current = batch.get_previous_header(&current)?;
				count += 1;
			}
			Ok(())
		})?;

		debug!(
			"validate_kernel_history: validated kernel root on {} headers",
			count,
		);

		Ok(())
	}

	/// Finds the "fork point" where header chain diverges from full block chain.
	/// If we are syncing this will correspond to the last full block where
	/// the next header is known but we do not yet have the full block.
	/// i.e. This is the last known full block and all subsequent blocks are missing.
	pub fn fork_point(&self) -> Result<BlockHeader, Error> {
		let body_head = self.head()?;
		let mut current = self.get_block_header(&body_head.hash())?;
		while !self.is_on_current_chain(&current, body_head).is_ok() {
			current = self.get_previous_header(&current)?;
		}
		Ok(current)
	}

	/// Compare fork point to our horizon.
	/// If beyond the horizon then we cannot sync via recent full blocks
	/// and we need a state (txhashset) sync.
	pub fn check_txhashset_needed(&self, fork_point: &BlockHeader) -> Result<bool, Error> {
		if self.archive_mode() {
			debug!("check_txhashset_needed: we are running with archive_mode=true, not needed");
			return Ok(false);
		}

		let header_head = self.header_head()?;
		let horizon = global::cut_through_horizon() as u64;
		Ok(fork_point.height < header_head.height.saturating_sub(horizon))
	}

	/// Clean the temporary sandbox folder
	pub fn clean_txhashset_sandbox(&self) {
		txhashset::clean_txhashset_folder(&self.get_tmp_dir());
	}

	/// Specific tmp dir.
	/// Normally it's ~/.grin/main/tmp for mainnet
	/// or ~/.grin/test/tmp for Testnet
	pub fn get_tmp_dir(&self) -> PathBuf {
		let mut tmp_dir = PathBuf::from(self.db_root.clone());
		tmp_dir = tmp_dir
			.parent()
			.expect("fail to get parent of db_root dir")
			.to_path_buf();
		tmp_dir.push("tmp");
		tmp_dir
	}

	/// Get a tmp file path in above specific tmp dir (create tmp dir if not exist)
	/// Delete file if tmp file already exists
	pub fn get_tmpfile_pathname(&self, tmpfile_name: String) -> PathBuf {
		let mut tmp = self.get_tmp_dir();
		if !tmp.exists() {
			if let Err(e) = fs::create_dir(tmp.clone()) {
				warn!("fail to create tmp folder on {:?}. err: {}", tmp, e);
			}
		}
		tmp.push(tmpfile_name);
		if tmp.exists() {
			if let Err(e) = fs::remove_file(tmp.clone()) {
				warn!("fail to clean existing tmp file: {:?}. err: {}", tmp, e);
			}
		}
		tmp
	}

	/// Writes a reading view on a txhashset state that's been provided to us.
	/// If we're willing to accept that new state, the data stream will be
	/// read as a zip file, unzipped and the resulting state files should be
	/// rewound to the provided indexes.
	pub fn txhashset_write(
		&self,
		h: Hash,
		txhashset_data: File,
		status: &dyn TxHashsetWriteStatus,
	) -> Result<bool, Error> {
		status.on_setup(None, None, None, None);

		// Initial check whether this txhashset is needed or not
		let fork_point = self.fork_point()?;
		if !self.check_txhashset_needed(&fork_point)? {
			warn!("txhashset_write: txhashset received but it's not needed! ignored.");
			return Err(Error::InvalidTxHashSet("not needed".to_owned()));
		}

		let header = match self.get_block_header(&h) {
			Ok(header) => header,
			Err(_) => {
				warn!("txhashset_write: cannot find block header");
				// This is a bannable reason
				return Ok(true);
			}
		};

		// Write txhashset to sandbox (in the Grin specific tmp dir)
		let sandbox_dir = self.get_tmp_dir();
		txhashset::clean_txhashset_folder(&sandbox_dir);
		txhashset::zip_write(sandbox_dir.clone(), txhashset_data.try_clone()?, &header)?;

		let mut txhashset = txhashset::TxHashSet::open(
			sandbox_dir
				.to_str()
				.expect("invalid sandbox folder")
				.to_owned(),
			self.store.clone(),
			Some(&header),
		)?;

		// Validate the full kernel history.
		// Check kernel MMR root for every block header.
		// Check NRD relative height rules for full kernel history.
		{
			self.validate_kernel_history(&header, &txhashset)?;

			let header_pmmr = self.header_pmmr.read();
			let batch = self.store.batch()?;
			txhashset.verify_kernel_pos_index(&self.genesis, &header_pmmr, &batch, None, None)?;
		}

		// all good, prepare a new batch and update all the required records
		debug!("txhashset_write: rewinding a 2nd time (writeable)");

		let mut header_pmmr = self.header_pmmr.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, batch| {
				let extension = &mut ext.extension;
				extension.rewind(&header, batch)?;

				// Validate the extension, generating the utxo_sum and kernel_sum.
				// Full validation, including rangeproofs and kernel signature verification.
				let (utxo_sum, kernel_sum) =
					extension.validate(&self.genesis, false, status, None, None, &header, None)?;

				// Save the block_sums (utxo_sum, kernel_sum) to the db for use later.
				batch.save_block_sums(
					&header.hash(),
					BlockSums {
						utxo_sum,
						kernel_sum,
					},
				)?;

				Ok(())
			},
		)?;

		debug!("txhashset_write: finished validating and rebuilding");

		status.on_save();

		// Save the new head to the db and rebuild the header by height index.
		{
			let tip = Tip::from_header(&header);
			batch.save_body_head(&tip)?;

			// Reset the body tail to the body head after a txhashset write
			batch.save_body_tail(&tip)?;
		}

		// Rebuild our output_pos index in the db based on fresh UTXO set.
		txhashset.init_output_pos_index(&header_pmmr, &batch)?;

		// Rebuild our NRD kernel_pos index based on recent kernel history.
		txhashset.init_recent_kernel_pos_index(&header_pmmr, &batch)?;

		// Commit all the changes to the db.
		batch.commit()?;

		debug!("txhashset_write: finished committing the batch (head etc.)");

		// Sandbox full validation ok, go to overwrite txhashset on db root
		{
			let mut txhashset_ref = self.txhashset.write();

			// Before overwriting, drop file handlers in underlying txhashset
			txhashset_ref.release_backend_files();

			// Move sandbox to overwrite
			txhashset.release_backend_files();
			txhashset::txhashset_replace(sandbox_dir, PathBuf::from(self.db_root.clone()))?;

			// Re-open on db root dir
			txhashset = txhashset::TxHashSet::open(
				self.db_root.clone(),
				self.store.clone(),
				Some(&header),
			)?;

			// Replace the chain txhashset with the newly built one.
			*txhashset_ref = txhashset;
		}

		debug!("txhashset_write: replaced our txhashset with the new one");

		status.on_done();

		Ok(false)
	}

	/// Cleanup old blocks from the db.
	/// Determine the cutoff height from the horizon and the current block height.
	/// *Only* runs if we are not in archive mode.
	fn remove_historical_blocks(
		&self,
		header_pmmr: &txhashset::PMMRHandle<BlockHeader>,
		archive_header: BlockHeader,
		batch: &store::Batch<'_>,
	) -> Result<(), Error> {
		if self.archive_mode() {
			return Ok(());
		}

		let mut horizon = global::cut_through_horizon() as u64;

		let head = batch.head()?;

		let tail = match batch.tail() {
			Ok(tail) => tail,
			Err(_) => Tip::from_header(&self.genesis),
		};

		let mut cutoff = head.height.saturating_sub(horizon);

		// TODO: Check this, compaction selects a different horizon
		// block from txhashset horizon/PIBD segmenter when using
		// Automated testing chain
		if archive_header.height < cutoff {
			cutoff = archive_header.height;
			horizon = head.height - archive_header.height;
		}

		debug!(
			"remove_historical_blocks: head height: {}, tail height: {}, horizon: {}, cutoff: {}",
			head.height, tail.height, horizon, cutoff,
		);

		if cutoff == 0 {
			return Ok(());
		}

		let mut count = 0;
		let tail_hash = header_pmmr.get_header_hash_by_height(head.height - horizon)?;
		let tail = batch.get_block_header(&tail_hash)?;

		// Remove old blocks (including short lived fork blocks) which height < tail.height
		for block in batch.blocks_iter()? {
			if block.header.height < tail.height {
				let _ = batch.delete_block(&block.hash());
				count += 1;
			}
		}

		batch.save_body_tail(&Tip::from_header(&tail))?;

		debug!(
			"remove_historical_blocks: removed {} blocks. tail height: {}",
			count, tail.height
		);

		Ok(())
	}

	/// Triggers chain compaction.
	///
	/// * compacts the txhashset based on current prune_list
	/// * removes historical blocks and associated data from the db (unless archive mode)
	///
	pub fn compact(&self) -> Result<(), Error> {
		// A node may be restarted multiple times in a short period of time.
		// We compact at most once per 60 blocks in this situation by comparing
		// current "head" and "tail" height to our cut-through horizon and
		// allowing an additional 60 blocks in height before allowing a further compaction.
		if let (Ok(tail), Ok(head)) = (self.tail(), self.head()) {
			let horizon = global::cut_through_horizon() as u64;
			let threshold = horizon.saturating_add(60);
			let next_compact = tail.height.saturating_add(threshold);
			if next_compact > head.height {
				debug!(
					"compact: skipping startup compaction (next at {})",
					next_compact
				);
				return Ok(());
			}
		}

		// Retrieve archive header here, so as not to attempt a read
		// lock while removing historical blocks
		let archive_header = self.txhashset_archive_header()?;

		// Take a write lock on the txhashet and start a new writeable db batch.
		let header_pmmr = self.header_pmmr.read();
		let mut txhashset = self.txhashset.write();
		let batch = self.store.batch()?;

		// Compact the txhashset itself (rewriting the pruned backend files).
		{
			let head_header = batch.head_header()?;
			let current_height = head_header.height;
			let horizon_height =
				current_height.saturating_sub(global::cut_through_horizon().into());
			let horizon_hash = header_pmmr.get_header_hash_by_height(horizon_height)?;
			let horizon_header = batch.get_block_header(&horizon_hash)?;

			txhashset.compact(&horizon_header, &batch)?;
		}

		// If we are not in archival mode remove historical blocks from the db.
		if !self.archive_mode() {
			self.remove_historical_blocks(&header_pmmr, archive_header, &batch)?;
		}

		// Make sure our output_pos index is consistent with the UTXO set.
		txhashset.init_output_pos_index(&header_pmmr, &batch)?;

		// TODO - Why is this part of chain compaction?
		// Rebuild our NRD kernel_pos index based on recent kernel history.
		txhashset.init_recent_kernel_pos_index(&header_pmmr, &batch)?;

		// Commit all the above db changes.
		batch.commit()?;

		Ok(())
	}

	/// returns the last n nodes inserted into the output sum tree
	pub fn get_last_n_output(&self, distance: u64) -> Vec<(Hash, OutputIdentifier)> {
		self.txhashset.read().last_n_output(distance)
	}

	/// as above, for rangeproofs
	pub fn get_last_n_rangeproof(&self, distance: u64) -> Vec<(Hash, RangeProof)> {
		self.txhashset.read().last_n_rangeproof(distance)
	}

	/// as above, for kernels
	pub fn get_last_n_kernel(&self, distance: u64) -> Vec<(Hash, TxKernel)> {
		self.txhashset.read().last_n_kernel(distance)
	}

	/// Return Commit's MMR position
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		Ok(self.txhashset.read().get_output_pos(commit)?)
	}

	/// outputs by insertion index
	pub fn unspent_outputs_by_pmmr_index(
		&self,
		start_index: u64,
		max_count: u64,
		max_pmmr_index: Option<u64>,
	) -> Result<(u64, u64, Vec<Output>), Error> {
		let txhashset = self.txhashset.read();
		let last_index = match max_pmmr_index {
			Some(i) => i,
			None => txhashset.output_mmr_size(),
		};
		let outputs = txhashset.outputs_by_pmmr_index(start_index, max_count, max_pmmr_index);
		let rangeproofs =
			txhashset.rangeproofs_by_pmmr_index(start_index, max_count, max_pmmr_index);
		if outputs.0 != rangeproofs.0 || outputs.1.len() != rangeproofs.1.len() {
			return Err(Error::TxHashSetErr(String::from(
				"Output and rangeproof sets don't match",
			)));
		}
		let mut output_vec: Vec<Output> = vec![];
		for (ref x, &y) in outputs.1.iter().zip(rangeproofs.1.iter()) {
			output_vec.push(Output::new(x.features, x.commitment(), y));
		}
		Ok((outputs.0, last_index, output_vec))
	}

	/// Return unspent outputs as above, but bounded between a particular range of blocks
	pub fn block_height_range_to_pmmr_indices(
		&self,
		start_block_height: u64,
		end_block_height: Option<u64>,
	) -> Result<(u64, u64), Error> {
		let end_block_height = match end_block_height {
			Some(h) => h,
			None => self.head_header()?.height,
		};
		// Return headers at the given heights
		let start_mmr_size = if start_block_height == 0 {
			0
		} else {
			self.get_header_by_height(start_block_height - 1)?
				.output_mmr_size + 1
		};
		let end_mmr_size = self.get_header_by_height(end_block_height)?.output_mmr_size;
		Ok((start_mmr_size, end_mmr_size))
	}

	/// Orphans pool size
	pub fn orphans_len(&self) -> usize {
		self.orphans.len()
	}

	/// Tip (head) of the block chain.
	pub fn head(&self) -> Result<Tip, Error> {
		self.store
			.head()
			.map_err(|e| Error::StoreErr(e, "chain head".to_owned()))
	}

	/// Tail of the block chain in this node after compact (cross-block cut-through)
	pub fn tail(&self) -> Result<Tip, Error> {
		self.store
			.tail()
			.map_err(|e| Error::StoreErr(e, "chain tail".to_owned()))
	}

	/// Tip (head) of the header chain.
	pub fn header_head(&self) -> Result<Tip, Error> {
		self.store
			.header_head()
			.map_err(|e| Error::StoreErr(e, "header head".to_owned()))
	}

	/// Block header for the chain head
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.store
			.head_header()
			.map_err(|e| Error::StoreErr(e, "chain head header".to_owned()))
	}

	/// Gets a block by hash
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		self.store
			.get_block(h)
			.map_err(|e| Error::StoreErr(e, "chain get block".to_owned()))
	}

	/// Gets a block header by hash
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		self.store
			.get_block_header(h)
			.map_err(|e| Error::StoreErr(e, "chain get header".to_owned()))
	}

	/// Get previous block header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.store
			.get_previous_header(header)
			.map_err(|e| Error::StoreErr(e, "chain get previous header".to_owned()))
	}

	/// Get block_sums by header hash.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		self.store
			.get_block_sums(h)
			.map_err(|e| Error::StoreErr(e, "chain get block_sums".to_owned()))
	}

	/// Gets the block header at the provided height.
	/// Note: Takes a read lock on the header_pmmr.
	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		let hash = self.get_header_hash_by_height(height)?;
		self.get_block_header(&hash)
	}

	/// Gets the header hash at the provided height.
	/// Note: Takes a read lock on the header_pmmr.
	fn get_header_hash_by_height(&self, height: u64) -> Result<Hash, Error> {
		self.header_pmmr.read().get_header_hash_by_height(height)
	}

	/// Gets the block header in which a given output appears in the txhashset.
	pub fn get_header_for_output(&self, commit: Commitment) -> Result<BlockHeader, Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		let (_, pos) = match txhashset.get_unspent(commit)? {
			Some(o) => o,
			None => return Err(Error::OutputNotFound),
		};
		let hash = header_pmmr.get_header_hash_by_height(pos.height)?;
		Ok(self.get_block_header(&hash)?)
	}

	/// Gets the kernel with a given excess and the block height it is included in.
	pub fn get_kernel_height(
		&self,
		excess: &Commitment,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<Option<(TxKernel, u64, u64)>, Error> {
		let head = self.head()?;

		if let (Some(min), Some(max)) = (min_height, max_height) {
			if min > max {
				return Ok(None);
			}
		}

		let min_index = match min_height {
			Some(0) => None,
			Some(h) => {
				if h > head.height {
					return Ok(None);
				}
				let header = self.get_header_by_height(h)?;
				let prev_header = self.get_previous_header(&header)?;
				Some(prev_header.kernel_mmr_size + 1)
			}
			None => None,
		};

		let max_index = match max_height {
			Some(h) => {
				if h > head.height {
					None
				} else {
					let header = self.get_header_by_height(h)?;
					Some(header.kernel_mmr_size)
				}
			}
			None => None,
		};

		let (kernel, mmr_index) = match self
			.txhashset
			.read()
			.find_kernel(&excess, min_index, max_index)
		{
			Some(k) => k,
			None => return Ok(None),
		};

		let header = self.get_header_for_kernel_index(mmr_index, min_height, max_height)?;

		Ok(Some((kernel, header.height, mmr_index)))
	}
	/// Gets the block header in which a given kernel mmr index appears in the txhashset.
	pub fn get_header_for_kernel_index(
		&self,
		kernel_mmr_index: u64,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<BlockHeader, Error> {
		let header_pmmr = self.header_pmmr.read();

		let mut min = min_height.unwrap_or(0).saturating_sub(1);
		let mut max = match max_height {
			Some(h) => h,
			None => self.head()?.height,
		};

		loop {
			let search_height = max - (max - min) / 2;
			let hash = header_pmmr.get_header_hash_by_height(search_height)?;
			let h = self.get_block_header(&hash)?;
			if search_height == 0 {
				return Ok(h);
			}
			let hash_prev = header_pmmr.get_header_hash_by_height(search_height - 1)?;
			let h_prev = self.get_block_header(&hash_prev)?;
			if kernel_mmr_index > h.kernel_mmr_size {
				min = search_height;
			} else if kernel_mmr_index < h_prev.kernel_mmr_size {
				max = search_height;
			} else {
				if kernel_mmr_index == h_prev.kernel_mmr_size {
					return Ok(h_prev);
				}
				return Ok(h);
			}
		}
	}

	/// Verifies the given block header is actually on the current chain.
	/// Checks the header_by_height index to verify the header is where we say
	/// it is
	fn is_on_current_chain<T: Into<Tip>>(&self, x: T, head: Tip) -> Result<(), Error> {
		let x: Tip = x.into();
		if x.height > head.height {
			return Err(Error::Other("not on current chain".to_string()));
		}

		if x.hash() == self.get_header_hash_by_height(x.height)? {
			Ok(())
		} else {
			Err(Error::Other("not on current chain".to_string()))
		}
	}

	/// Gets multiple headers at the provided heights.
	/// Note: This is based on the provided sync_head to support syncing against a fork.
	pub fn get_locator_hashes(&self, sync_head: Tip, heights: &[u64]) -> Result<Vec<Hash>, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		txhashset::header_extending_readonly(&mut header_pmmr, &self.store(), |ext, batch| {
			let header = batch.get_block_header(&sync_head.hash())?;
			self.rewind_and_apply_header_fork(&header, ext, batch)?;

			let hashes = heights
				.iter()
				.filter_map(|h| ext.get_header_hash_by_height(*h))
				.collect();

			Ok(hashes)
		})
	}

	/// Builds an iterator on blocks starting from the current chain head and
	/// running backward. Specialized to return information pertaining to block
	/// difficulty calculation (timestamp and previous difficulties).
	pub fn difficulty_iter(&self) -> Result<store::DifficultyIter<'_>, Error> {
		let head = self.head()?;
		let store = self.store.clone();
		Ok(store::DifficultyIter::from(head.last_block_h, store))
	}

	/// Check whether we have a block without reading it
	pub fn block_exists(&self, h: Hash) -> Result<bool, Error> {
		self.store
			.block_exists(&h)
			.map_err(|e| Error::StoreErr(e, "chain block exists".to_owned()))
	}
}

fn setup_head(
	genesis: &Block,
	store: &store::ChainStore,
	header_pmmr: &mut txhashset::PMMRHandle<BlockHeader>,
	txhashset: &mut txhashset::TxHashSet,
) -> Result<(), Error> {
	let mut batch = store.batch()?;

	// Apply the genesis header to header MMR.
	{
		if batch.get_block_header(&genesis.hash()).is_err() {
			batch.save_block_header(&genesis.header)?;
		}

		if header_pmmr.size == 0 {
			txhashset::header_extending(header_pmmr, &mut batch, |ext, _| {
				ext.apply_header(&genesis.header)
			})?;
		}
	}

	// Make sure our header PMMR is consistent with header_head from db if it exists.
	// If header_head is missing in db then use head of header PMMR.
	if let Ok(head) = batch.header_head() {
		header_pmmr.init_head(&head)?;
		txhashset::header_extending(header_pmmr, &mut batch, |ext, batch| {
			let header = batch.get_block_header(&head.hash())?;
			ext.rewind(&header)
		})?;
	} else {
		let hash = header_pmmr.head_hash()?;
		let header = batch.get_block_header(&hash)?;
		batch.save_header_head(&Tip::from_header(&header))?;
	}

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
				let mut pibd_in_progress = false;
				let header = {
					let head = batch.get_block_header(&head.last_block_h)?;
					let pibd_tip = store.pibd_head()?;
					let pibd_head = batch.get_block_header(&pibd_tip.last_block_h)?;
					if pibd_head.height > head.height {
						pibd_in_progress = true;
						pibd_head
					} else {
						head
					}
				};

				let res = txhashset::extending(header_pmmr, txhashset, &mut batch, |ext, batch| {
					// If we're still downloading via PIBD, don't worry about sums and validations just yet
					// We still want to rewind to the last completed block to ensure a consistent state
					if pibd_in_progress {
						debug!(
							"init: PIBD appears to be in progress at height {}, hash {}, not validating, will attempt to continue",
							header.height,
							header.hash()
						);
						return Ok(());
					}

					pipe::rewind_and_apply_fork(&header, ext, batch, &|_| Ok(()))?;

					let extension = &mut ext.extension;

					extension.validate_roots(&header)?;

					// now check we have the "block sums" for the block in question
					// if we have no sums (migrating an existing node) we need to go
					// back to the txhashset and sum the outputs and kernels
					if header.height > 0 && batch.get_block_sums(&header.hash()).is_err() {
						debug!(
							"init: building (missing) block sums for {} @ {}",
							header.height,
							header.hash()
						);

						// Do a full (and slow) validation of the txhashset extension
						// to calculate the utxo_sum and kernel_sum at this block height.
						let (utxo_sum, kernel_sum) =
							extension.validate_kernel_sums(&genesis.header, &header)?;

						// Save the block_sums to the db for use later.
						batch.save_block_sums(
							&header.hash(),
							BlockSums {
								utxo_sum,
								kernel_sum,
							},
						)?;
					}

					debug!(
						"init: rewinding and validating before we start... {} at {}",
						header.hash(),
						header.height,
					);
					Ok(())
				});

				if res.is_ok() {
					break;
				} else {
					// We may have corrupted the MMR backend files last time we stopped the
					// node. If this happens we rewind to the previous header,
					// delete the "bad" block and try again.
					let prev_header = batch.get_block_header(&head.prev_block_h)?;

					txhashset::extending(header_pmmr, txhashset, &mut batch, |ext, batch| {
						pipe::rewind_and_apply_fork(&prev_header, ext, batch, &|_| Ok(()))
					})?;

					// Now "undo" the latest block and forget it ever existed.
					// We will request it from a peer during sync as necessary.
					{
						let _ = batch.delete_block(&header.hash());
						head = Tip::from_header(&prev_header);
						batch.save_body_head(&head)?;
					}
				}
			}
		}
		Err(NotFoundErr(_)) => {
			let mut sums = BlockSums::default();

			// Save the genesis header with a "zero" header_root.
			// We will update this later once we have the correct header_root.
			batch.save_block(&genesis)?;
			batch.save_spent_index(&genesis.hash(), &vec![])?;
			batch.save_body_head(&Tip::from_header(&genesis.header))?;

			if !genesis.kernels().is_empty() {
				let (utxo_sum, kernel_sum) = (sums, genesis as &dyn Committed).verify_kernel_sums(
					genesis.header.overage(),
					genesis.header.total_kernel_offset(),
				)?;
				sums = BlockSums {
					utxo_sum,
					kernel_sum,
				};
			}
			txhashset::extending(header_pmmr, txhashset, &mut batch, |ext, batch| {
				ext.extension
					.apply_block(&genesis, ext.header_extension, batch)
			})?;

			// Save the block_sums to the db for use later.
			batch.save_block_sums(&genesis.hash(), sums)?;

			info!("init: saved genesis: {:?}", genesis.hash());
		}
		Err(e) => return Err(Error::StoreErr(e, "chain init load head".to_owned())),
	};
	batch.commit()?;
	Ok(())
}
