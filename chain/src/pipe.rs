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

//! Implementation of the chain block acceptance (or refusal) pipeline.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use chrono::prelude::Utc;
use chrono::Duration;

use chain::OrphanBlockPool;
use core::consensus;
use core::core::hash::{Hash, Hashed};
use core::core::verifier_cache::VerifierCache;
use core::core::{Block, BlockHeader};
use core::global;
use core::pow::Difficulty;
use error::{Error, ErrorKind};
use grin_store;
use store;
use txhashset;
use types::{Options, Tip};
use util::LOGGER;

use failure::ResultExt;

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext {
	/// The options
	pub opts: Options,
	/// The store
	pub store: Arc<store::ChainStore>,
	/// The head
	pub head: Tip,
	/// The POW verification function
	pub pow_verifier: fn(&BlockHeader, u8) -> bool,
	/// MMR sum tree states
	pub txhashset: Arc<RwLock<txhashset::TxHashSet>>,
	/// Recently processed blocks to avoid double-processing
	pub block_hashes_cache: Arc<RwLock<VecDeque<Hash>>>,
	/// Recent orphan blocks to avoid double-processing
	pub orphans: Arc<OrphanBlockPool>,
}

// Check if this block is the next block *immediately*
// after our current chain head.
fn is_next_block(header: &BlockHeader, ctx: &mut BlockContext) -> bool {
	header.previous == ctx.head.last_block_h
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain. Returns the new chain head if
/// updated.
pub fn process_block(
	b: &Block,
	ctx: &mut BlockContext,
	verifier_cache: Arc<RwLock<VerifierCache>>,
) -> Result<Option<Tip>, Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	debug!(
		LOGGER,
		"pipe: process_block {} at {} with {} inputs, {} outputs, {} kernels",
		b.hash(),
		b.header.height,
		b.inputs().len(),
		b.outputs().len(),
		b.kernels().len(),
	);

	// First thing we do is take a write lock on the txhashset.
	// We may receive the same block from multiple peers simultaneously.
	// We want to process the first one fully to avoid redundant work
	// processing the duplicates.
	let txhashset = ctx.txhashset.clone();
	let mut txhashset = txhashset.write().unwrap();

	// Update head now that we are in the lock.
	ctx.head = ctx.store.head()?;

	// Fast in-memory checks to avoid re-processing a block we recently processed.
	{
		// Check if we have recently processed this block (via ctx chain head).
		check_known_head(&b.header, ctx)?;

		// Check if we have recently processed this block (via block_hashes_cache).
		check_known_cache(&b.header, ctx)?;

		// Check if this block is already know due it being in the current set of orphan blocks.
		check_known_orphans(&b.header, ctx)?;
	}

	// Check our header itself is actually valid before proceeding any further.
	validate_header(&b.header, ctx)?;

	// Check if are processing the "next" block relative to the current chain head.
	if is_next_block(&b.header, ctx) {
		// If this is the "next" block then either -
		//   * common case where we process blocks sequentially.
		//   * special case where this is the first fast sync full block
		// Either way we can proceed (and we know the block is new and unprocessed).
	} else {
		// Check we have *this* block in the store.
		// Stop if we have processed this block previously (it is in the store).
		// This is more expensive than the earlier check_known() as we hit the store.
		check_known_store(&b.header, ctx)?;

		// Check existing MMR (via rewind) to see if this block is known to us already.
		// This should catch old blocks before we check to see if they appear to be
		// orphaned due to compacting/pruning on a fast-sync node.
		// This is more expensive than check_known_store() as we rewind the txhashset.
		// But we only incur the cost of the rewind if this is an earlier block on the same chain.
		check_known_mmr(&b.header, ctx, &mut txhashset)?;

		// At this point it looks like this is a new block that we have not yet processed.
		// Check we have the *previous* block in the store.
		// If we do not then treat this block as an orphan.
		check_prev_store(&b.header, ctx)?;
	}

	// Validate the block itself.
	// Taking advantage of the verifier_cache for
	// rangeproofs and kernel signatures.
	validate_block(b, ctx, verifier_cache)?;

	// Begin a new batch as we may begin modifying the db at this point.
	let store = ctx.store.clone();
	let mut batch = store.batch()?;

	// Start a chain extension unit of work dependent on the success of the
	// internal validation and saving operations
	txhashset::extending(&mut txhashset, &mut batch, |mut extension| {
		// First we rewind the txhashset extension if necessary
		// to put it into a consistent state for validating the block.
		// We can skip this step if the previous header is the latest header we saw.
		if is_next_block(&b.header, ctx) {
			// No need to rewind if we are processing the next block.
		} else {
			// Rewind the re-apply blocks on the forked chain to
			// put the txhashset in the correct forked state
			// (immediately prior to this new block).
			rewind_and_apply_fork(b, ctx.store.clone(), extension)?;
		}

		// Check any coinbase being spent have matured sufficiently.
		// This needs to be done within the context of a potentially
		// rewound txhashset extension to reflect chain state prior
		// to applying the new block.
		verify_coinbase_maturity(b, &mut extension)?;

		// Apply the block to the txhashset state.
		// Validate the txhashset roots and sizes against the block header.
		// Block is invalid if there are any discrepencies.
		apply_block_to_txhashset(b, &mut extension)?;

		// If applying this block does not increase the work on the chain then
		// we know we have not yet updated the chain to produce a new chain head.
		if !block_has_more_work(&b.header, &ctx.head) {
			extension.force_rollback();
		}

		Ok(())
	})?;

	trace!(
		LOGGER,
		"pipe: process_block: {} at {} is valid, save and append.",
		b.hash(),
		b.header.height,
	);

	// Add the newly accepted block and header to our index.
	add_block(b, &mut batch)?;

	// Update the chain head in the index (if necessary)
	let res = update_head(b, &ctx, &mut batch)?;

	// Commit the batch to store all updates to the db/index.
	batch.commit()?;

	// Return the new chain tip if we added work, or
	// None if this block has not added work.
	Ok(res)
}

/// Process the block header.
/// This is only ever used during sync and uses a context based on sync_head.
pub fn sync_block_header(
	bh: &BlockHeader,
	sync_ctx: &mut BlockContext,
	header_ctx: &mut BlockContext,
	batch: &mut store::Batch,
) -> Result<Option<Tip>, Error> {
	debug!(
		LOGGER,
		"pipe: sync_block_header: {} at {}",
		bh.hash(),
		bh.height
	);

	validate_header(&bh, sync_ctx)?;
	add_block_header(bh, batch)?;

	// now update the header_head (if new header with most work) and the sync_head
	// (always)
	update_header_head(bh, header_ctx, batch)?;
	update_sync_head(bh, sync_ctx, batch)
}

/// Process block header as part of "header first" block propagation.
/// We validate the header but we do not store it or update header head based
/// on this. We will update these once we get the block back after requesting
/// it.
pub fn process_block_header(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	debug!(
		LOGGER,
		"pipe: process_block_header at {} [{}]",
		bh.height,
		bh.hash()
	); // keep this

	check_header_known(bh.hash(), ctx)?;
	validate_header(&bh, ctx)
}

/// Quick in-memory check to fast-reject any block header we've already handled
/// recently. Keeps duplicates from the network in check.
/// ctx here is specific to the header_head (tip of the header chain)
fn check_header_known(bh: Hash, ctx: &mut BlockContext) -> Result<(), Error> {
	if bh == ctx.head.last_block_h || bh == ctx.head.prev_block_h {
		return Err(ErrorKind::Unfit("already known".to_string()).into());
	}
	Ok(())
}

/// Quick in-memory check to fast-reject any block handled recently.
/// Keeps duplicates from the network in check.
/// Checks against the last_block_h and prev_block_h of the chain head.
fn check_known_head(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	let bh = header.hash();
	if bh == ctx.head.last_block_h || bh == ctx.head.prev_block_h {
		return Err(ErrorKind::Unfit("already known in head".to_string()).into());
	}
	Ok(())
}

/// Quick in-memory check to fast-reject any block handled recently.
/// Keeps duplicates from the network in check.
/// Checks against the cache of recently processed block hashes.
fn check_known_cache(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	let cache = ctx.block_hashes_cache.read().unwrap();
	if cache.contains(&header.hash()) {
		return Err(ErrorKind::Unfit("already known in cache".to_string()).into());
	}
	Ok(())
}

/// Check if this block is in the set of known orphans.
fn check_known_orphans(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	if ctx.orphans.contains(&header.hash()) {
		Err(ErrorKind::Unfit("already known in orphans".to_string()).into())
	} else {
		Ok(())
	}
}

// Check if this block is in the store already.
fn check_known_store(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	match ctx.store.block_exists(&header.hash()) {
		Ok(true) => {
			if header.height < ctx.head.height.saturating_sub(50) {
				// TODO - we flag this as an "abusive peer" but only in the case
				// where we have the full block in our store.
				// So this is not a particularly exhaustive check.
				Err(ErrorKind::OldBlock.into())
			} else {
				Err(ErrorKind::Unfit("already known in store".to_string()).into())
			}
		}
		Ok(false) => {
			// Not yet processed this block, we can proceed.
			Ok(())
		}
		Err(e) => {
			return Err(ErrorKind::StoreErr(e, "pipe get this block".to_owned()).into());
		}
	}
}

// Check we have the *previous* block in the store.
// Note: not just the header but the full block itself.
// We cannot assume we can use the chain head for this
// as we may be dealing with a fork (with less work currently).
fn check_prev_store(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	match ctx.store.block_exists(&header.previous) {
		Ok(true) => {
			// We have the previous block in the store, so we can proceed.
			Ok(())
		}
		Ok(false) => {
			// We do not have the previous block in the store.
			// We have not yet processed the previous block so
			// this block is an orphan (for now).
			Err(ErrorKind::Orphan.into())
		}
		Err(e) => Err(ErrorKind::StoreErr(e, "pipe get previous".to_owned()).into()),
	}
}

// If we are processing an "old" block then
// we can quickly check if it already exists
// on our current longest chain (we have already processes it).
// First check the header matches via current height index.
// Then peek directly into the MMRs at the appropriate pos.
// We can avoid a full rewind in this case.
fn check_known_mmr(
	header: &BlockHeader,
	ctx: &mut BlockContext,
	write_txhashset: &mut txhashset::TxHashSet,
) -> Result<(), Error> {
	// No point checking the MMR if this block is not earlier in the chain.
	if header.height > ctx.head.height {
		return Ok(());
	}

	// Use "header by height" index to look at current most work chain.
	// Header is not "known if the header differs at the given height.
	let local_header = ctx.store.get_header_by_height(header.height)?;
	if local_header.hash() != header.hash() {
		return Ok(());
	}

	// Rewind the txhashset to the given block and validate
	// roots and sizes against the header.
	// If everything matches then this is a "known" block
	// and we do not need to spend any more effort
	txhashset::extending_readonly(write_txhashset, |extension| {
		extension.rewind(header)?;

		// We want to return an error here (block already known)
		// if we *successfully validate the MMR roots and sizes.
		if extension.validate_roots(header).is_ok() && extension.validate_sizes(header).is_ok() {
			// TODO - determine if block is more than 50 blocks old
			// and return specific OldBlock error.
			// Or pull OldBlock (abusive peer) out into separate processing step.

			return Err(ErrorKind::Unfit("already known on most work chain".to_string()).into());
		}

		// If we get here then we have *not* seen this block before
		// and we should continue processing the block.
		Ok(())
	})?;

	Ok(())
}

/// First level of block validation that only needs to act on the block header
/// to make it as cheap as possible. The different validations are also
/// arranged by order of cost to have as little DoS surface as possible.
fn validate_header(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	// check version, enforces scheduled hard fork
	if !consensus::valid_header_version(header.height, header.version) {
		error!(
			LOGGER,
			"Invalid block header version received ({}), maybe update Grin?", header.version
		);
		return Err(ErrorKind::InvalidBlockVersion(header.version).into());
	}

	// TODO: remove CI check from here somehow
	if header.timestamp > Utc::now() + Duration::seconds(12 * (consensus::BLOCK_TIME_SEC as i64))
		&& !global::is_automated_testing_mode()
	{
		// refuse blocks more than 12 blocks intervals in future (as in bitcoin)
		// TODO add warning in p2p code if local time is too different from peers
		return Err(ErrorKind::InvalidBlockTime.into());
	}

	if !ctx.opts.contains(Options::SKIP_POW) {
		let shift = header.pow.cuckoo_sizeshift();
		// size shift can either be larger than the minimum on the primary PoW
		// or equal to the seconday PoW size shift
		if shift != consensus::SECOND_POW_SIZESHIFT && global::min_sizeshift() > shift {
			return Err(ErrorKind::LowSizeshift.into());
		}
		// primary PoW must have a scaling factor of 1
		if shift != consensus::SECOND_POW_SIZESHIFT && header.pow.scaling_difficulty != 1 {
			return Err(ErrorKind::InvalidScaling.into());
		}
		if !(ctx.pow_verifier)(header, shift) {
			error!(
				LOGGER,
				"pipe: validate_header bad cuckoo shift size {}", shift
			);
			return Err(ErrorKind::InvalidPow.into());
		}
	}

	// first I/O cost, better as late as possible
	let prev = match ctx.store.get_block_header(&header.previous) {
		Ok(prev) => prev,
		Err(grin_store::Error::NotFoundErr(_)) => return Err(ErrorKind::Orphan.into()),
		Err(e) => {
			return Err(
				ErrorKind::StoreErr(e, format!("previous header {}", header.previous)).into(),
			)
		}
	};

	// make sure this header has a height exactly one higher than the previous
	// header
	if header.height != prev.height + 1 {
		return Err(ErrorKind::InvalidBlockHeight.into());
	}

	// TODO - get rid of the automated testing mode check here somehow
	if header.timestamp <= prev.timestamp && !global::is_automated_testing_mode() {
		// prevent time warp attacks and some timestamp manipulations by forcing strict
		// time progression (but not in CI mode)
		return Err(ErrorKind::InvalidBlockTime.into());
	}

	// verify the proof of work and related parameters
	// at this point we have a previous block header
	// we know the height increased by one
	// so now we can check the total_difficulty increase is also valid
	// check the pow hash shows a difficulty at least as large
	// as the target difficulty
	if !ctx.opts.contains(Options::SKIP_POW) {
		if header.total_difficulty() <= prev.total_difficulty() {
			return Err(ErrorKind::DifficultyTooLow.into());
		}

		let target_difficulty = header.total_difficulty() - prev.total_difficulty();

		if header.pow.to_difficulty() < target_difficulty {
			return Err(ErrorKind::DifficultyTooLow.into());
		}

		// explicit check to ensure we are not below the minimum difficulty
		// we will also check difficulty based on next_difficulty later on
		if target_difficulty < Difficulty::one() {
			return Err(ErrorKind::DifficultyTooLow.into());
		}

		// explicit check to ensure total_difficulty has increased by exactly
		// the _network_ difficulty of the previous block
		// (during testnet1 we use _block_ difficulty here)
		let diff_iter = store::DifficultyIter::from(header.previous, ctx.store.clone());
		let network_difficulty = consensus::next_difficulty(diff_iter)
			.context(ErrorKind::Other("network difficulty".to_owned()))?;
		if target_difficulty != network_difficulty.clone() {
			error!(
				LOGGER,
				"validate_header: header target difficulty {} != {}",
				target_difficulty.to_num(),
				network_difficulty.to_num()
			);
			return Err(ErrorKind::WrongTotalDifficulty.into());
		}
	}

	Ok(())
}

fn validate_block(
	block: &Block,
	ctx: &mut BlockContext,
	verifier_cache: Arc<RwLock<VerifierCache>>,
) -> Result<(), Error> {
	let prev = ctx.store.get_block_header(&block.header.previous)?;
	block
		.validate(
			&prev.total_kernel_offset,
			&prev.total_kernel_sum,
			verifier_cache,
		)
		.map_err(|e| ErrorKind::InvalidBlockProof(e))?;
	Ok(())
}

/// Verify the block is not attempting to spend coinbase outputs
/// before they have sufficiently matured.
/// Note: requires a txhashset extension.
fn verify_coinbase_maturity(block: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	ext.verify_coinbase_maturity(&block.inputs(), block.header.height)?;
	Ok(())
}

/// Fully validate the block by applying it to the txhashset extension.
/// Check both the txhashset roots and sizes are correct after applying the block.
fn apply_block_to_txhashset(block: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	ext.apply_block(block)?;
	ext.validate_roots(&block.header)?;
	ext.validate_sizes(&block.header)?;
	Ok(())
}

/// Officially adds the block to our chain.
fn add_block(b: &Block, batch: &mut store::Batch) -> Result<(), Error> {
	// Save the block itself to the db (via the batch).
	batch
		.save_block(b)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save block".to_owned()))?;

	// Build the block_input_bitmap, save to the db (via the batch) and cache locally.
	batch.build_and_cache_block_input_bitmap(&b)?;
	Ok(())
}

/// Officially adds the block header to our header chain.
fn add_block_header(bh: &BlockHeader, batch: &mut store::Batch) -> Result<(), Error> {
	batch
		.save_block_header(bh)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save header".to_owned()).into())
}

/// Directly updates the head if we've just appended a new block to it or handle
/// the situation where we've just added enough work to have a fork with more
/// work than the head.
fn update_head(b: &Block, ctx: &BlockContext, batch: &store::Batch) -> Result<Option<Tip>, Error> {
	// if we made a fork with more work than the head (which should also be true
	// when extending the head), update it
	if block_has_more_work(&b.header, &ctx.head) {
		// update the block height index
		batch
			.setup_height(&b.header, &ctx.head)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe setup height".to_owned()))?;

		// in sync mode, only update the "body chain", otherwise update both the
		// "header chain" and "body chain", updating the header chain in sync resets
		// all additional "future" headers we've received
		let tip = Tip::from_block(&b.header);
		if ctx.opts.contains(Options::SYNC) {
			batch
				.save_body_head(&tip)
				.map_err(|e| ErrorKind::StoreErr(e, "pipe save body".to_owned()))?;
		} else {
			batch
				.save_head(&tip)
				.map_err(|e| ErrorKind::StoreErr(e, "pipe save head".to_owned()))?;
		}
		debug!(
			LOGGER,
			"pipe: chain head {} @ {}",
			b.hash(),
			b.header.height
		);
		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

// Whether the provided block totals more work than the chain tip
fn block_has_more_work(header: &BlockHeader, tip: &Tip) -> bool {
	let block_tip = Tip::from_block(header);
	block_tip.total_difficulty > tip.total_difficulty
}

/// Update the sync head so we can keep syncing from where we left off.
fn update_sync_head(
	bh: &BlockHeader,
	ctx: &mut BlockContext,
	batch: &mut store::Batch,
) -> Result<Option<Tip>, Error> {
	let tip = Tip::from_block(bh);
	batch
		.save_sync_head(&tip)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save sync head".to_owned()))?;
	ctx.head = tip.clone();
	debug!(LOGGER, "sync head {} @ {}", bh.hash(), bh.height);
	Ok(Some(tip))
}

fn update_header_head(
	bh: &BlockHeader,
	ctx: &mut BlockContext,
	batch: &mut store::Batch,
) -> Result<Option<Tip>, Error> {
	let tip = Tip::from_block(bh);
	if tip.total_difficulty > ctx.head.total_difficulty {
		batch
			.save_header_head(&tip)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe save header head".to_owned()))?;
		ctx.head = tip.clone();
		debug!(LOGGER, "header head {} @ {}", bh.hash(), bh.height);
		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

/// Utility function to handle forks. From the forked block, jump backward
/// to find to fork root. Rewind the txhashset to the root and apply all the
/// forked blocks prior to the one being processed to set the txhashset in
/// the expected state.
pub fn rewind_and_apply_fork(
	b: &Block,
	store: Arc<store::ChainStore>,
	ext: &mut txhashset::Extension,
) -> Result<(), Error> {
	// extending a fork, first identify the block where forking occurred
	// keeping the hashes of blocks along the fork
	let mut current = b.header.previous;
	let mut fork_hashes = vec![];
	loop {
		let curr_header = store.get_block_header(&current)?;

		if let Ok(_) = store.is_on_current_chain(&curr_header) {
			break;
		} else {
			fork_hashes.insert(0, (curr_header.height, curr_header.hash()));
			current = curr_header.previous;
		}
	}

	let forked_header = store.get_block_header(&current)?;

	trace!(
		LOGGER,
		"rewind_and_apply_fork @ {} [{}], was @ {} [{}]",
		forked_header.height,
		forked_header.hash(),
		b.header.height,
		b.header.hash()
	);

	// Rewind the txhashset state back to the block where we forked from the most work chain.
	ext.rewind(&forked_header)?;

	trace!(
		LOGGER,
		"rewind_and_apply_fork: blocks on fork: {:?}",
		fork_hashes,
	);

	// Now re-apply all blocks on this fork.
	for (_, h) in fork_hashes {
		let fb = store
			.get_block(&h)
			.map_err(|e| ErrorKind::StoreErr(e, format!("getting forked blocks")))?;
		ext.apply_block(&fb)?;
	}
	Ok(())
}
