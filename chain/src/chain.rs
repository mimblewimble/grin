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

use crate::core::core::hash::{Hash, Hashed, ZERO_HASH};
use crate::core::core::merkle_proof::MerkleProof;
use crate::core::core::verifier_cache::VerifierCache;
use crate::core::core::{
	Block, BlockHeader, BlockSums, Committed, Output, OutputIdentifier, Transaction, TxKernel,
	TxKernelEntry,
};
use crate::core::global;
use crate::core::pow;
use crate::core::ser::{ProtocolVersion, Readable, StreamingReader};
use crate::error::{Error, ErrorKind};
use crate::pipe;
use crate::store;
use crate::txhashset;
use crate::txhashset::{PMMRHandle, TxHashSet};
use crate::types::{
	BlockStatus, ChainAdapter, NoStatus, Options, OutputMMRPosition, Tip, TxHashSetRoots,
	TxHashsetWriteStatus,
};
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::RwLock;
use grin_store::Error::NotFoundErr;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
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
		let mut orphans = self.orphans.write();
		let mut height_idx = self.height_idx.write();
		height_idx
			.remove(height)
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
	sync_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
	verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	// POW verification function
	pow_verifier: fn(&BlockHeader) -> Result<(), pow::Error>,
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
		verifier_cache: Arc<RwLock<dyn VerifierCache>>,
		archive_mode: bool,
	) -> Result<Chain, Error> {
		let store = Arc::new(store::ChainStore::new(&db_root)?);

		// open the txhashset, creating a new one if necessary
		let mut txhashset = txhashset::TxHashSet::open(db_root.clone(), store.clone(), None)?;

		let mut header_pmmr =
			PMMRHandle::new(&db_root, "header", "header_head", false, true, None)?;
		let mut sync_pmmr = PMMRHandle::new(&db_root, "header", "sync_head", false, true, None)?;

		setup_head(
			&genesis,
			&store,
			&mut header_pmmr,
			&mut sync_pmmr,
			&mut txhashset,
		)?;
		Chain::log_heads(&store)?;

		Ok(Chain {
			db_root,
			store,
			adapter,
			orphans: Arc::new(OrphanBlockPool::new()),
			txhashset: Arc::new(RwLock::new(txhashset)),
			header_pmmr: Arc::new(RwLock::new(header_pmmr)),
			sync_pmmr: Arc::new(RwLock::new(sync_pmmr)),
			pow_verifier,
			verifier_cache,
			archive_mode,
			genesis: genesis.header.clone(),
		})
	}

	/// Return our shared header MMR handle.
	pub fn header_pmmr(&self) -> Arc<RwLock<PMMRHandle<BlockHeader>>> {
		self.header_pmmr.clone()
	}

	/// Return our shared txhashset instance.
	pub fn txhashset(&self) -> Arc<RwLock<TxHashSet>> {
		self.txhashset.clone()
	}

	/// Shared store instance.
	pub fn store(&self) -> Arc<store::ChainStore> {
		self.store.clone()
	}

	fn log_heads(store: &store::ChainStore) -> Result<(), Error> {
		let head = store.head()?;
		debug!(
			"init: head: {} @ {} [{}]",
			head.total_difficulty.to_num(),
			head.height,
			head.last_block_h,
		);

		let header_head = store.header_head()?;
		debug!(
			"init: header_head: {} @ {} [{}]",
			header_head.total_difficulty.to_num(),
			header_head.height,
			header_head.last_block_h,
		);

		let sync_head = store.get_sync_head()?;
		debug!(
			"init: sync_head: {} @ {} [{}]",
			sync_head.total_difficulty.to_num(),
			sync_head.height,
			sync_head.last_block_h,
		);

		Ok(())
	}

	/// Reset sync_head to current header_head.
	/// We do this when we first transition to header_sync to ensure we extend
	/// the "sync" header MMR from a known consistent state and to ensure we track
	/// the header chain correctly at the fork point.
	pub fn reset_sync_head(&self) -> Result<Tip, Error> {
		let batch = self.store.batch()?;
		batch.reset_sync_head()?;
		let head = batch.get_sync_head()?;
		batch.commit()?;
		Ok(head)
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

	fn determine_status(&self, head: Option<Tip>, prev_head: Tip) -> BlockStatus {
		// We have more work if the chain head is updated.
		let is_more_work = head.is_some();

		let mut is_next_block = false;
		let mut reorg_depth = None;
		if let Some(head) = head {
			if head.prev_block_h == prev_head.last_block_h {
				is_next_block = true;
			} else {
				reorg_depth = Some(prev_head.height.saturating_sub(head.height) + 1);
			}
		}

		match (is_more_work, is_next_block) {
			(true, true) => BlockStatus::Next,
			(true, false) => BlockStatus::Reorg(reorg_depth.unwrap_or(0)),
			(false, _) => BlockStatus::Fork,
		}
	}

	/// Attempt to add a new block to the chain.
	/// Returns true if it has been added to the longest chain
	/// or false if it has added to a fork (or orphan?).
	fn process_block_single(&self, b: Block, opts: Options) -> Result<Option<Tip>, Error> {
		let (maybe_new_head, prev_head) = {
			let mut header_pmmr = self.header_pmmr.write();
			let mut txhashset = self.txhashset.write();
			let batch = self.store.batch()?;
			let mut ctx = self.new_ctx(opts, batch, &mut header_pmmr, &mut txhashset)?;

			let prev_head = ctx.batch.head()?;

			let maybe_new_head = pipe::process_block(&b, &mut ctx);

			// We have flushed txhashset extension changes to disk
			// but not yet committed the batch.
			// A node shutdown at this point can be catastrophic...
			// We prevent this via the stop_lock (see above).
			if let Ok(_) = maybe_new_head {
				ctx.batch.commit()?;
			}

			// release the lock and let the batch go before post-processing
			(maybe_new_head, prev_head)
		};

		match maybe_new_head {
			Ok(head) => {
				let status = self.determine_status(head.clone(), prev_head);

				// notifying other parts of the system of the update
				self.adapter.block_accepted(&b, status, opts);

				Ok(head)
			}
			Err(e) => match e.kind() {
				ErrorKind::Orphan => {
					let block_hash = b.hash();
					let orphan = Orphan {
						block: b,
						opts: opts,
						added: Instant::now(),
					};

					&self.orphans.add(orphan);

					debug!(
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
						"Block {} at {} is unfit at this time: {}",
						b.hash(),
						b.header.height,
						msg
					);
					Err(ErrorKind::Unfit(msg.clone()).into())
				}
				_ => {
					info!(
						"Rejected block {} at {}: {:?}",
						b.hash(),
						b.header.height,
						e
					);
					Err(ErrorKind::Other(format!("{:?}", e).to_owned()).into())
				}
			},
		}
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
	pub fn sync_block_headers(&self, headers: &[BlockHeader], opts: Options) -> Result<(), Error> {
		let mut sync_pmmr = self.sync_pmmr.write();
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		// Sync the chunk of block headers, updating sync_head as necessary.
		{
			let batch = self.store.batch()?;
			let mut ctx = self.new_ctx(opts, batch, &mut sync_pmmr, &mut txhashset)?;
			pipe::sync_block_headers(headers, &mut ctx)?;
			ctx.batch.commit()?;
		}

		// Now "process" the last block header, updating header_head to match sync_head.
		if let Some(header) = headers.last() {
			let batch = self.store.batch()?;
			let mut ctx = self.new_ctx(opts, batch, &mut header_pmmr, &mut txhashset)?;
			pipe::process_block_header(header, &mut ctx)?;
			ctx.batch.commit()?;
		}

		Ok(())
	}

	fn new_ctx<'a>(
		&self,
		opts: Options,
		batch: store::Batch<'a>,
		header_pmmr: &'a mut txhashset::PMMRHandle<BlockHeader>,
		txhashset: &'a mut txhashset::TxHashSet,
	) -> Result<pipe::BlockContext<'a>, Error> {
		Ok(pipe::BlockContext {
			opts,
			pow_verifier: self.pow_verifier,
			verifier_cache: self.verifier_cache.clone(),
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
	pub fn check_orphans(&self, mut height: u64) {
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

			if let Some(orphans) = self.orphans.remove_by_height(&height) {
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

	/// TODO - where do we call this from? And do we need a rewind first?
	/// For the given commitment find the unspent output and return the
	/// associated Return an error if the output does not exist or has been
	/// spent. This querying is done in a way that is consistent with the
	/// current chain state, specifically the current winning (valid, most
	/// work) fork.
	pub fn is_unspent(&self, output_ref: &OutputIdentifier) -> Result<OutputMMRPosition, Error> {
		let txhashset = self.txhashset.read();
		txhashset.is_unspent(output_ref)
	}

	/// Validate the tx against the current UTXO set.
	pub fn validate_tx(&self, tx: &Transaction) -> Result<(), Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		txhashset::utxo_view(&header_pmmr, &txhashset, |utxo| {
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
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		txhashset::utxo_view(&header_pmmr, &txhashset, |utxo| {
			utxo.verify_coinbase_maturity(&tx.inputs(), height)?;
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

		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		// Now create an extension from the txhashset and validate against the
		// latest block header. Rewind the extension to the specified header to
		// ensure the view is consistent.
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |mut ext| {
			pipe::rewind_and_apply_fork(&header, &mut ext)?;
			let ref mut extension = ext.extension;
			extension.validate(&self.genesis, fast_validation, &NoStatus)?;
			Ok(())
		})
	}

	/// Sets the txhashset roots on a brand new block by applying the block on
	/// the current txhashset state.
	pub fn set_txhashset_roots(&self, b: &mut Block) -> Result<(), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();

		let (prev_root, roots, sizes) =
			txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext| {
				let previous_header = ext.batch().get_previous_header(&b.header)?;
				pipe::rewind_and_apply_fork(&previous_header, ext)?;

				let ref mut extension = ext.extension;
				let ref mut header_extension = ext.header_extension;

				// Retrieve the header root before we apply the new block
				let prev_root = header_extension.root()?;

				// Apply the latest block to the chain state via the extension.
				extension.apply_block(b)?;

				Ok((prev_root, extension.roots()?, extension.sizes()))
			})?;

		// Set the prev_root on the header.
		b.header.prev_root = prev_root;

		// Set the output, rangeproof and kernel MMR roots.
		b.header.output_root = roots.output_root;
		b.header.range_proof_root = roots.rproof_root;
		b.header.kernel_root = roots.kernel_root;

		// Set the output and kernel MMR sizes.
		{
			// Carefully destructure these correctly...
			let (output_mmr_size, _, kernel_mmr_size) = sizes;
			b.header.output_mmr_size = output_mmr_size;
			b.header.kernel_mmr_size = kernel_mmr_size;
		}

		Ok(())
	}

	/// Return a Merkle proof for the given commitment from the store.
	pub fn get_merkle_proof(
		&self,
		output: &OutputIdentifier,
		header: &BlockHeader,
	) -> Result<MerkleProof, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let merkle_proof =
			txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext| {
				pipe::rewind_and_apply_fork(&header, ext)?;
				let ref mut extension = ext.extension;
				extension.merkle_proof(output)
			})?;

		Ok(merkle_proof)
	}

	/// Return a merkle proof valid for the current output pmmr state at the
	/// given pos
	pub fn get_merkle_proof_for_pos(&self, commit: Commitment) -> Result<MerkleProof, Error> {
		let mut txhashset = self.txhashset.write();
		txhashset.merkle_proof(commit)
	}

	/// Returns current txhashset roots.
	pub fn get_txhashset_roots(&self) -> TxHashSetRoots {
		self.txhashset.read().roots()
	}

	/// Provides a reading view into the current kernel state.
	pub fn kernel_data_read(&self) -> Result<File, Error> {
		let txhashset = self.txhashset.read();
		txhashset::rewindable_kernel_view(&txhashset, |view| view.kernel_data_read())
	}

	/// Writes kernels provided to us (via a kernel data download).
	/// Currently does not write these to disk and simply deserializes
	/// the provided data.
	/// TODO - Write this data to disk and validate the rebuilt kernel MMR.
	pub fn kernel_data_write(&self, reader: &mut dyn Read) -> Result<(), Error> {
		let mut count = 0;
		let mut stream = StreamingReader::new(reader, ProtocolVersion::local());
		while let Ok(_kernel) = TxKernelEntry::read(&mut stream) {
			count += 1;
		}

		debug!("kernel_data_write: read {} kernels", count);

		Ok(())
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
			let mut header_pmmr = self.header_pmmr.write();
			let mut txhashset = self.txhashset.write();
			txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext| {
				pipe::rewind_and_apply_fork(&header, ext)?;
				let ref mut extension = ext.extension;
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
		txhashset::rewindable_kernel_view(&txhashset, |view| {
			while current.height > 0 {
				view.rewind(&current)?;
				view.validate_root()?;
				current = view.batch().get_previous_header(&current)?;
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

	/// Rebuild the sync MMR based on current header_head.
	/// We rebuild the sync MMR when first entering sync mode so ensure we
	/// have an MMR we can safely rewind based on the headers received from a peer.
	pub fn rebuild_sync_mmr(&self, head: &Tip) -> Result<(), Error> {
		let mut sync_pmmr = self.sync_pmmr.write();
		let mut batch = self.store.batch()?;
		let sync_head = batch.get_sync_head()?;
		let header = batch.get_block_header(&head.hash())?;
		txhashset::header_extending(&mut sync_pmmr, &sync_head, &mut batch, |extension| {
			pipe::rewind_and_apply_header_fork(&header, extension)?;
			Ok(())
		})?;
		batch.commit()?;
		Ok(())
	}

	/// Check chain status whether a txhashset downloading is needed
	pub fn check_txhashset_needed(
		&self,
		caller: String,
		hashes: &mut Option<Vec<Hash>>,
	) -> Result<bool, Error> {
		let horizon = global::cut_through_horizon() as u64;
		let body_head = self.head()?;
		let header_head = self.header_head()?;
		let sync_head = self.get_sync_head()?;

		debug!(
			"{}: body_head - {}, {}, header_head - {}, {}, sync_head - {}, {}",
			caller,
			body_head.last_block_h,
			body_head.height,
			header_head.last_block_h,
			header_head.height,
			sync_head.last_block_h,
			sync_head.height,
		);

		if body_head.total_difficulty >= header_head.total_difficulty {
			debug!(
				"{}: no need txhashset. header_head.total_difficulty: {} <= body_head.total_difficulty: {}",
				caller, header_head.total_difficulty, body_head.total_difficulty,
			);
			return Ok(false);
		}

		let mut oldest_height = 0;
		let mut oldest_hash = ZERO_HASH;

		// Start with body_head (head of the full block chain)
		let mut current = self.get_block_header(&body_head.last_block_h);
		if current.is_err() {
			error!(
				"{}: body_head not found in chain db: {} at {}",
				caller, body_head.last_block_h, body_head.height,
			);
			return Ok(false);
		}

		//
		// TODO - Investigate finding the "common header" by comparing header_mmr and
		// sync_mmr (bytes will be identical up to the common header).
		//
		// Traverse back through the full block chain from body head until we find a header
		// that "is on current chain", which is the "fork point" between existing header chain
		// and full block chain.
		while let Ok(header) = current {
			// break out of the while loop when we find a header common
			// between the header chain and the current body chain
			if let Ok(_) = self.is_on_current_chain(&header) {
				oldest_height = header.height;
				oldest_hash = header.hash();
				break;
			}

			current = self.get_previous_header(&header);
		}

		// Traverse back through the header chain from header_head back to this fork point.
		// These are the blocks that we need to request in body sync (we have the header but not the full block).
		if let Some(hs) = hashes {
			let mut h = self.get_block_header(&header_head.last_block_h);
			while let Ok(header) = h {
				if header.height <= oldest_height {
					break;
				}
				hs.push(header.hash());
				h = self.get_previous_header(&header);
			}
		}

		if oldest_height < header_head.height.saturating_sub(horizon) {
			if oldest_hash != ZERO_HASH {
				// this is the normal case. for example:
				// body head height is 1 (and not a fork), oldest_height will be 1
				// body head height is 0 (a typical fresh node), oldest_height will be 0
				// body head height is 10,001 (but at a fork with depth 1), oldest_height will be 10,000
				// body head height is 10,005 (but at a fork with depth 5), oldest_height will be 10,000
				debug!(
					"{}: need a state sync for txhashset. oldest block which is not on local chain: {} at {}",
					caller, oldest_hash, oldest_height,
				);
			} else {
				// this is the abnormal case, when is_on_current_chain() always return Err, and even for genesis block.
				error!("{}: corrupted storage? state sync is needed", caller);
			}
			Ok(true)
		} else {
			Ok(false)
		}
	}

	/// Clean the temporary sandbox folder
	pub fn clean_txhashset_sandbox(&self) {
		txhashset::clean_txhashset_folder(&self.get_tmp_dir());
	}

	/// Specific tmp dir.
	/// Normally it's ~/.grin/main/tmp for mainnet
	/// or ~/.grin/floo/tmp for floonet
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
		status.on_setup();

		// Initial check whether this txhashset is needed or not
		let mut hashes: Option<Vec<Hash>> = None;
		if !self.check_txhashset_needed("txhashset_write".to_owned(), &mut hashes)? {
			warn!("txhashset_write: txhashset received but it's not needed! ignored.");
			return Err(ErrorKind::InvalidTxHashSet("not needed".to_owned()).into());
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

		// Validate the full kernel history (kernel MMR root for every block header).
		self.validate_kernel_history(&header, &txhashset)?;

		// all good, prepare a new batch and update all the required records
		debug!("txhashset_write: rewinding a 2nd time (writeable)");

		let mut header_pmmr = self.header_pmmr.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(&mut header_pmmr, &mut txhashset, &mut batch, |ext| {
			let extension = &mut ext.extension;
			extension.rewind(&header)?;

			// Validate the extension, generating the utxo_sum and kernel_sum.
			// Full validation, including rangeproofs and kernel signature verification.
			let (utxo_sum, kernel_sum) = extension.validate(&self.genesis, false, status)?;

			// Save the block_sums (utxo_sum, kernel_sum) to the db for use later.
			extension.batch.save_block_sums(
				&header.hash(),
				&BlockSums {
					utxo_sum,
					kernel_sum,
				},
			)?;

			Ok(())
		})?;

		debug!("txhashset_write: finished validating and rebuilding");

		status.on_save();

		// Save the new head to the db and rebuild the header by height index.
		{
			let tip = Tip::from_header(&header);
			batch.save_body_head(&tip)?;

			// Reset the body tail to the body head after a txhashset write
			batch.save_body_tail(&tip)?;
		}

		// Rebuild our output_pos index in the db based on current UTXO set.
		txhashset.rebuild_height_pos_index(&header_pmmr, &mut batch)?;

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
			txhashset::txhashset_replace(sandbox_dir.clone(), PathBuf::from(self.db_root.clone()))?;

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

		// Check for any orphan blocks and process them based on the new chain state.
		self.check_orphans(header.height + 1);

		status.on_done();

		Ok(false)
	}

	/// Cleanup old blocks from the db.
	/// Determine the cutoff height from the horizon and the current block height.
	/// *Only* runs if we are not in archive mode.
	fn remove_historical_blocks(
		&self,
		header_pmmr: &txhashset::PMMRHandle<BlockHeader>,
		batch: &mut store::Batch<'_>,
	) -> Result<(), Error> {
		if self.archive_mode {
			return Ok(());
		}

		let horizon = global::cut_through_horizon() as u64;
		let head = batch.head()?;

		let tail = match batch.tail() {
			Ok(tail) => tail,
			Err(_) => Tip::from_header(&self.genesis),
		};

		let cutoff = head.height.saturating_sub(horizon);

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
		// here b is a block
		for (_, b) in batch.blocks_iter()? {
			if b.header.height < tail.height {
				let _ = batch.delete_block(&b.hash());
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
			debug!(
				"compact: head: {}, tail: {}, diff: {}, horizon: {}",
				head.height,
				tail.height,
				head.height.saturating_sub(tail.height),
				horizon
			);
			if tail.height.saturating_add(threshold) > head.height {
				debug!("compact: skipping compaction - threshold is 60 blocks beyond horizon.");
				return Ok(());
			}
		}

		// Take a write lock on the txhashet and start a new writeable db batch.
		let header_pmmr = self.header_pmmr.read();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;

		// Compact the txhashset itself (rewriting the pruned backend files).
		{
			let head_header = batch.head_header()?;
			let current_height = head_header.height;
			let horizon_height =
				current_height.saturating_sub(global::cut_through_horizon().into());
			let horizon_hash = header_pmmr.get_header_hash_by_height(horizon_height)?;
			let horizon_header = batch.get_block_header(&horizon_hash)?;

			txhashset.compact(&horizon_header, &mut batch)?;
		}

		// Rebuild our output_pos index in the db based on current UTXO set.
		txhashset.rebuild_height_pos_index(&header_pmmr, &mut batch)?;

		// If we are not in archival mode remove historical blocks from the db.
		if !self.archive_mode {
			self.remove_historical_blocks(&header_pmmr, &mut batch)?;
		}

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
	pub fn get_last_n_kernel(&self, distance: u64) -> Vec<(Hash, TxKernelEntry)> {
		self.txhashset.read().last_n_kernel(distance)
	}

	/// Return Commit's MMR position
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		Ok(self.txhashset.read().get_output_pos(commit)?)
	}

	/// outputs by insertion index
	pub fn unspent_outputs_by_insertion_index(
		&self,
		start_index: u64,
		max: u64,
	) -> Result<(u64, u64, Vec<Output>), Error> {
		let txhashset = self.txhashset.read();
		let max_index = txhashset.highest_output_insertion_index();
		let outputs = txhashset.outputs_by_insertion_index(start_index, max);
		let rangeproofs = txhashset.rangeproofs_by_insertion_index(start_index, max);
		if outputs.0 != rangeproofs.0 || outputs.1.len() != rangeproofs.1.len() {
			return Err(ErrorKind::TxHashSetErr(String::from(
				"Output and rangeproof sets don't match",
			))
			.into());
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

	/// Tip (head) of the block chain.
	pub fn head(&self) -> Result<Tip, Error> {
		self.store
			.head()
			.map_err(|e| ErrorKind::StoreErr(e, "chain head".to_owned()).into())
	}

	/// Tail of the block chain in this node after compact (cross-block cut-through)
	pub fn tail(&self) -> Result<Tip, Error> {
		self.store
			.tail()
			.map_err(|e| ErrorKind::StoreErr(e, "chain tail".to_owned()).into())
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

	/// Get previous block header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.store
			.get_previous_header(header)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get previous header".to_owned()).into())
	}

	/// Get block_sums by header hash.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		self.store
			.get_block_sums(h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain get block_sums".to_owned()).into())
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

	/// Migrate the index 'commitment -> output_pos' to index 'commitment -> (output_pos, block_height)'
	/// Note: should only be called when Node start-up, for database migration from the old version.
	pub fn rebuild_height_for_pos(&self) -> Result<(), Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		let mut outputs_pos = txhashset.get_all_output_pos()?;
		let total_outputs = outputs_pos.len();
		if total_outputs == 0 {
			debug!("rebuild_height_for_pos: nothing to be rebuilt");
			return Ok(());
		} else {
			debug!(
				"rebuild_height_for_pos: rebuilding {} output_pos's height...",
				total_outputs
			);
		}
		outputs_pos.sort_by(|a, b| a.1.cmp(&b.1));

		let max_height = {
			let head = self.head()?;
			head.height
		};

		let batch = self.store.batch()?;
		// clear it before rebuilding
		batch.clear_output_pos_height()?;

		let mut i = 0;
		for search_height in 0..max_height {
			let hash = header_pmmr.get_header_hash_by_height(search_height + 1)?;
			let h = batch.get_block_header(&hash)?;
			while i < total_outputs {
				let (commit, pos) = outputs_pos[i];
				if pos > h.output_mmr_size {
					// Note: MMR position is 1-based and not 0-based, so here must be '>' instead of '>='
					break;
				}
				batch.save_output_pos_height(&commit, pos, h.height)?;
				trace!("rebuild_height_for_pos: {:?}", (commit, pos, h.height));
				i += 1;
			}
		}

		// clear the output_pos since now it has been replaced by the new index
		batch.clear_output_pos()?;

		batch.commit()?;
		debug!("rebuild_height_for_pos: done");
		Ok(())
	}

	/// Gets the block header in which a given output appears in the txhashset.
	pub fn get_header_for_output(
		&self,
		output_ref: &OutputIdentifier,
	) -> Result<BlockHeader, Error> {
		let header_pmmr = self.header_pmmr.read();
		let txhashset = self.txhashset.read();
		let output_pos = txhashset.is_unspent(output_ref)?;
		let hash = header_pmmr.get_header_hash_by_height(output_pos.height)?;
		Ok(self.get_block_header(&hash)?)
	}

	/// Gets the kernel with a given excess and the block height it is included in.
	pub fn get_kernel_height(
		&self,
		excess: &Commitment,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<Option<(TxKernel, u64, u64)>, Error> {
		let min_index = match min_height {
			Some(h) => Some(self.get_header_by_height(h - 1)?.kernel_mmr_size + 1),
			None => None,
		};

		let max_index = match max_height {
			Some(h) => Some(self.get_header_by_height(h)?.kernel_mmr_size),
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
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		let chain_header = self.get_header_by_height(header.height)?;
		if chain_header.hash() == header.hash() {
			Ok(())
		} else {
			Err(ErrorKind::Other(format!("not on current chain")).into())
		}
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
	pub fn difficulty_iter(&self) -> Result<store::DifficultyIter<'_>, Error> {
		let head = self.head()?;
		let store = self.store.clone();
		Ok(store::DifficultyIter::from(head.last_block_h, store))
	}

	/// Check whether we have a block without reading it
	pub fn block_exists(&self, h: Hash) -> Result<bool, Error> {
		self.store
			.block_exists(&h)
			.map_err(|e| ErrorKind::StoreErr(e, "chain block exists".to_owned()).into())
	}
}

fn setup_head(
	genesis: &Block,
	store: &store::ChainStore,
	header_pmmr: &mut txhashset::PMMRHandle<BlockHeader>,
	sync_pmmr: &mut txhashset::PMMRHandle<BlockHeader>,
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

				let res = txhashset::extending(header_pmmr, txhashset, &mut batch, |ext| {
					pipe::rewind_and_apply_fork(&header, ext)?;

					let ref mut extension = ext.extension;

					extension.validate_roots()?;

					// now check we have the "block sums" for the block in question
					// if we have no sums (migrating an existing node) we need to go
					// back to the txhashset and sum the outputs and kernels
					if header.height > 0 && extension.batch.get_block_sums(&header.hash()).is_err()
					{
						debug!(
							"init: building (missing) block sums for {} @ {}",
							header.height,
							header.hash()
						);

						// Do a full (and slow) validation of the txhashset extension
						// to calculate the utxo_sum and kernel_sum at this block height.
						let (utxo_sum, kernel_sum) =
							extension.validate_kernel_sums(&genesis.header)?;

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
					// node. If this appears to be the case revert the head to the previous
					// header and try again
					let prev_header = batch.get_block_header(&head.prev_block_h)?;
					let _ = batch.delete_block(&header.hash());
					head = Tip::from_header(&prev_header);
					batch.save_body_head(&head)?;
				}
			}
		}
		Err(NotFoundErr(_)) => {
			let mut sums = BlockSums::default();

			// Save the genesis header with a "zero" header_root.
			// We will update this later once we have the correct header_root.
			batch.save_block_header(&genesis.header)?;
			batch.save_block(&genesis)?;

			let tip = Tip::from_header(&genesis.header);

			// Save these ahead of time as we need head and header_head to be initialized
			// with *something* when creating a txhashset extension below.
			batch.save_body_head(&tip)?;
			batch.save_header_head(&tip)?;

			if genesis.kernels().len() > 0 {
				let (utxo_sum, kernel_sum) = (sums, genesis as &dyn Committed).verify_kernel_sums(
					genesis.header.overage(),
					genesis.header.total_kernel_offset(),
				)?;
				sums = BlockSums {
					utxo_sum,
					kernel_sum,
				};
			}
			txhashset::header_extending(header_pmmr, &tip, &mut batch, |extension| {
				extension.apply_header(&genesis.header)?;
				Ok(())
			})?;
			txhashset::header_extending(sync_pmmr, &tip, &mut batch, |extension| {
				extension.apply_header(&genesis.header)?;
				Ok(())
			})?;
			txhashset::extending(header_pmmr, txhashset, &mut batch, |ext| {
				let ref mut extension = ext.extension;
				extension.apply_block(&genesis)?;
				extension.validate_roots()?;
				extension.validate_sizes()?;
				Ok(())
			})?;

			// Save the block_sums to the db for use later.
			batch.save_block_sums(&genesis.hash(), &sums)?;

			info!("init: saved genesis: {:?}", genesis.hash());
		}
		Err(e) => return Err(ErrorKind::StoreErr(e, "chain init load head".to_owned()))?,
	};

	// Check we have the header corresponding to the header_head.
	// If not then something is corrupted and we should reset our header_head.
	// Either way we want to reset sync_head to match header_head.
	let head = batch.head()?;
	let header_head = batch.header_head()?;
	if batch.get_block_header(&header_head.last_block_h).is_ok() {
		// Reset sync_head to be consistent with current header_head.
		batch.reset_sync_head()?;
	} else {
		// Reset both header_head and sync_head to be consistent with current head.
		warn!(
			"setup_head: header missing for {}, {}, resetting header_head and sync_head to head: {}, {}",
			header_head.last_block_h,
			header_head.height,
			head.last_block_h,
			head.height,
		);
		batch.reset_header_head()?;
		batch.reset_sync_head()?;
	}

	batch.commit()?;

	Ok(())
}
