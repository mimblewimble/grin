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

use std::sync::{Arc, RwLock};

use chrono::prelude::Utc;
use chrono::Duration;

use lru_cache::LruCache;

use chain::OrphanBlockPool;
use core::consensus;
use core::core::hash::{Hash, Hashed};
use core::core::verifier_cache::VerifierCache;
use core::core::Committed;
use core::core::{Block, BlockHeader, BlockSums};
use core::global;
use core::pow::{self, Difficulty};
use error::{Error, ErrorKind};
use grin_store;
use store;
use txhashset;
use types::{Options, Tip};
use util::LOGGER;

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext<'a> {
	/// The options
	pub opts: Options,
	/// The pow verifier to use when processing a block.
	pub pow_verifier: fn(&BlockHeader, u8) -> Result<(), pow::Error>,
	/// The active txhashset (rewindable MMRs) to use for block processing.
	pub txhashset: &'a mut txhashset::TxHashSet,
	/// The active batch to use for block processing.
	pub batch: store::Batch<'a>,

	/// Recently processed blocks to avoid double-processing
	pub block_hashes_cache: Arc<RwLock<LruCache<Hash, bool>>>,
	/// The verifier cache (caching verifier for rangeproofs and kernel signatures)
	pub verifier_cache: Arc<RwLock<VerifierCache>>,
	/// Recent orphan blocks to avoid double-processing
	pub orphans: Arc<OrphanBlockPool>,
}

// Check if this block is the next block *immediately*
// after our current chain head.
fn is_next_block(header: &BlockHeader, head: &Tip) -> bool {
	header.previous == head.last_block_h
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain.
/// Returns new head if chain head updated.
pub fn process_block(b: &Block, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
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

	// Fast in-memory checks to avoid re-processing a block we recently processed.
	{
		// Check if we have recently processed this block (via ctx chain head).
		check_known_head(&b.header, ctx)?;

		// Check if we have recently processed this block (via block_hashes_cache).
		check_known_cache(&b.header, ctx)?;

		// Check if this block is already know due it being in the current set of orphan blocks.
		check_known_orphans(&b.header, ctx)?;
	}

	// Header specific processing.
	handle_block_header(&b.header, ctx)?;

	// Check if are processing the "next" block relative to the current chain head.
	let head = ctx.batch.head()?;
	if is_next_block(&b.header, &head) {
		// If this is the "next" block then either -
		//   * common case where we process blocks sequentially.
		//   * special case where this is the first fast sync full block
		// Either way we can proceed (and we know the block is new and unprocessed).
	} else {
		// Check we have *this* block in the store.
		// Stop if we have processed this block previously (it is in the store).
		// This is more expensive than the earlier check_known() as we hit the store.
		check_known_store(&b.header, ctx)?;

		// At this point it looks like this is a new block that we have not yet processed.
		// Check we have the *previous* block in the store.
		// If we do not then treat this block as an orphan.
		check_prev_store(&b.header, &mut ctx.batch)?;
	}

	// Validate the block itself, make sure it is internally consistent.
	// Use the verifier_cache for verifying rangeproofs and kernel signatures.
	validate_block(b, ctx)?;

	// Start a chain extension unit of work dependent on the success of the
	// internal validation and saving operations
	txhashset::extending(&mut ctx.txhashset, &mut ctx.batch, |mut extension| {
		// First we rewind the txhashset extension if necessary
		// to put it into a consistent state for validating the block.
		// We can skip this step if the previous header is the latest header we saw.
		if is_next_block(&b.header, &head) {
			// No need to rewind if we are processing the next block.
		} else {
			// Rewind the re-apply blocks on the forked chain to
			// put the txhashset in the correct forked state
			// (immediately prior to this new block).
			rewind_and_apply_fork(b, extension)?;
		}

		// Check any coinbase being spent have matured sufficiently.
		// This needs to be done within the context of a potentially
		// rewound txhashset extension to reflect chain state prior
		// to applying the new block.
		verify_coinbase_maturity(b, &mut extension)?;

		// Validate the block against the UTXO set.
		validate_utxo(b, &mut extension)?;

		// Using block_sums (utxo_sum, kernel_sum) for the previous block from the db
		// we can verify_kernel_sums across the full UTXO sum and full kernel sum
		// accounting for inputs/outputs/kernels in this new block.
		// We know there are no double-spends etc. if this verifies successfully.
		verify_block_sums(b, &mut extension)?;

		// Apply the block to the txhashset state.
		// Validate the txhashset roots and sizes against the block header.
		// Block is invalid if there are any discrepencies.
		apply_block_to_txhashset(b, &mut extension)?;

		// If applying this block does not increase the work on the chain then
		// we know we have not yet updated the chain to produce a new chain head.
		let head = extension.batch.head()?;
		if !has_more_work(&b.header, &head) {
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
	add_block(b, ctx)?;

	// Update the chain head (and header_head) if total work is increased.
	let res = {
		let _ = update_header_head(&b.header, ctx)?;
		let res = update_head(b, ctx)?;
		res
	};
	Ok(res)
}

/// Process the block header.
/// This is only ever used during sync and uses a context based on sync_head.
pub fn sync_block_headers(
	headers: &Vec<BlockHeader>,
	ctx: &mut BlockContext,
) -> Result<Option<Tip>, Error> {
	if let Some(header) = headers.first() {
		debug!(
			LOGGER,
			"pipe: sync_block_headers: {} headers from {} at {}",
			headers.len(),
			header.hash(),
			header.height,
		);
	} else {
		return Ok(None);
	}

	let all_known = if let Some(last_header) = headers.last() {
		ctx.batch.get_block_header(&last_header.hash()).is_ok()
	} else {
		false
	};

	if !all_known {
		for header in headers {
			handle_block_header(header, ctx)?;
		}
	}

	// Update header_head (if most work) and sync_head (regardless) in all cases,
	// even if we already know all the headers.
	// This avoids the case of us getting into an infinite loop with sync_head never
	// progressing.
	// We only need to do this once at the end of this batch of headers.
	if let Some(header) = headers.last() {
		// Update sync_head regardless of total work.
		update_sync_head(header, &mut ctx.batch)?;

		// Update header_head (but only if this header increases our total known work).
		// i.e. Only if this header is now the head of the current "most work" chain.
		let res = update_header_head(header, ctx)?;
		Ok(res)
	} else {
		Ok(None)
	}
}

fn handle_block_header(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	validate_header(header, ctx)?;
	add_block_header(header, ctx)?;
	Ok(())
}

/// Process block header as part of "header first" block propagation.
/// We validate the header but we do not store it or update header head based
/// on this. We will update these once we get the block back after requesting
/// it.
pub fn process_block_header(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	debug!(
		LOGGER,
		"pipe: process_block_header: {} at {}",
		header.hash(),
		header.height,
	); // keep this

	check_header_known(header, ctx)?;
	validate_header(header, ctx)?;
	Ok(())
}

/// Quick in-memory check to fast-reject any block header we've already handled
/// recently. Keeps duplicates from the network in check.
/// ctx here is specific to the header_head (tip of the header chain)
fn check_header_known(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	let header_head = ctx.batch.header_head()?;
	if header.hash() == header_head.last_block_h || header.hash() == header_head.prev_block_h {
		return Err(ErrorKind::Unfit("header already known".to_string()).into());
	}
	Ok(())
}

/// Quick in-memory check to fast-reject any block handled recently.
/// Keeps duplicates from the network in check.
/// Checks against the last_block_h and prev_block_h of the chain head.
fn check_known_head(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	let head = ctx.batch.head()?;
	let bh = header.hash();
	if bh == head.last_block_h || bh == head.prev_block_h {
		return Err(ErrorKind::Unfit("already known in head".to_string()).into());
	}
	Ok(())
}

/// Quick in-memory check to fast-reject any block handled recently.
/// Keeps duplicates from the network in check.
/// Checks against the cache of recently processed block hashes.
fn check_known_cache(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	let mut cache = ctx.block_hashes_cache.write().unwrap();
	if cache.contains_key(&header.hash()) {
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
	match ctx.batch.block_exists(&header.hash()) {
		Ok(true) => {
			let head = ctx.batch.head()?;
			if header.height < head.height.saturating_sub(50) {
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
fn check_prev_store(header: &BlockHeader, batch: &mut store::Batch) -> Result<(), Error> {
	match batch.block_exists(&header.previous) {
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
		if !header.pow.is_primary() && !header.pow.is_secondary() {
			return Err(ErrorKind::InvalidSizeshift.into());
		}
		let shift = header.pow.cuckoo_sizeshift();
		if !(ctx.pow_verifier)(header, shift).is_ok() {
			error!(
				LOGGER,
				"pipe: error validating header with cuckoo shift size {}", shift
			);
			return Err(ErrorKind::InvalidPow.into());
		}
	}

	// first I/O cost, better as late as possible
	let prev = match ctx.batch.get_block_header(&header.previous) {
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
		let child_batch = ctx.batch.child()?;
		let diff_iter = store::DifficultyIter::from_batch(header.previous, child_batch);
		let next_header_info = consensus::next_difficulty(header.height, diff_iter);
		if target_difficulty != next_header_info.difficulty {
			info!(
				LOGGER,
				"validate_header: header target difficulty {} != {}",
				target_difficulty.to_num(),
				next_header_info.difficulty.to_num()
			);
			return Err(ErrorKind::WrongTotalDifficulty.into());
		}
		// check the secondary PoW scaling factor if applicable
		if header.pow.scaling_difficulty != next_header_info.secondary_scaling {
			return Err(ErrorKind::InvalidScaling.into());
		}
	}

	Ok(())
}

fn validate_block(block: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	let prev = ctx.batch.get_block_header(&block.header.previous)?;
	block
		.validate(&prev.total_kernel_offset, ctx.verifier_cache.clone())
		.map_err(|e| ErrorKind::InvalidBlockProof(e))?;
	Ok(())
}

/// TODO - This can move into the utxo_view.
/// Verify the block is not attempting to spend coinbase outputs
/// before they have sufficiently matured.
/// Note: requires a txhashset extension.
fn verify_coinbase_maturity(block: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	ext.verify_coinbase_maturity(&block.inputs(), block.header.height)?;
	Ok(())
}

/// Some "real magick" verification logic.
/// The (BlockSums, Block) tuple implements Committed...
/// This allows us to verify kernel sums across the full utxo and kernel sets
/// based on block_sums of previous block, accounting for the inputs|outputs|kernels
/// of the new block.
fn verify_block_sums(b: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	// Retrieve the block_sums for the previous block.
	let block_sums = ext.batch.get_block_sums(&b.header.previous)?;

	// Overage is based purely on the new block.
	// Previous block_sums have taken all previous overage into account.
	let overage = b.header.overage();

	// Offset on the other hand is the total kernel offset from the new block.
	let offset = b.header.total_kernel_offset();

	// Verify the kernel sums for the block_sums with the new block applied.
	let (utxo_sum, kernel_sum) =
		(block_sums, b as &Committed).verify_kernel_sums(overage, offset)?;

	// Save the new block_sums for the new block to the db via the batch.
	ext.batch.save_block_sums(
		&b.header.hash(),
		&BlockSums {
			utxo_sum,
			kernel_sum,
		},
	)?;

	Ok(())
}

/// Fully validate the block by applying it to the txhashset extension.
/// Check both the txhashset roots and sizes are correct after applying the block.
fn apply_block_to_txhashset(block: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	ext.apply_block(block)?;
	ext.validate_roots()?;
	ext.validate_sizes()?;
	Ok(())
}

/// Officially adds the block to our chain.
fn add_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	// Save the block itself to the db (via the batch).
	ctx.batch
		.save_block(b)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save block".to_owned()))?;

	// Build the block_input_bitmap, save to the db (via the batch) and cache locally.
	ctx.batch.build_and_cache_block_input_bitmap(&b)?;
	Ok(())
}

/// Officially adds the block header to our header chain.
fn add_block_header(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.batch
		.save_block_header(bh)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save header".to_owned()).into())
}

/// Directly updates the head if we've just appended a new block to it or handle
/// the situation where we've just added enough work to have a fork with more
/// work than the head.
fn update_head(b: &Block, ctx: &BlockContext) -> Result<Option<Tip>, Error> {
	// if we made a fork with more work than the head (which should also be true
	// when extending the head), update it
	let head = ctx.batch.head()?;
	if has_more_work(&b.header, &head) {
		// Update the block height index based on this new head.
		ctx.batch
			.setup_height(&b.header, &head)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe setup height".to_owned()))?;

		let tip = Tip::from_block(&b.header);

		ctx.batch
			.save_body_head(&tip)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe save body".to_owned()))?;

		debug!(
			LOGGER,
			"pipe: head updated to {} at {}", tip.last_block_h, tip.height
		);

		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

// Whether the provided block totals more work than the chain tip
fn has_more_work(header: &BlockHeader, head: &Tip) -> bool {
	header.total_difficulty() > head.total_difficulty
}

/// Update the sync head so we can keep syncing from where we left off.
fn update_sync_head(bh: &BlockHeader, batch: &mut store::Batch) -> Result<(), Error> {
	let tip = Tip::from_block(bh);
	batch
		.save_sync_head(&tip)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save sync head".to_owned()))?;
	debug!(LOGGER, "sync head {} @ {}", bh.hash(), bh.height);
	Ok(())
}

/// Update the header head if this header has most work.
fn update_header_head(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	let header_head = ctx.batch.header_head()?;
	if has_more_work(&bh, &header_head) {
		let tip = Tip::from_block(bh);
		ctx.batch
			.save_header_head(&tip)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe save header head".to_owned()))?;

		debug!(
			LOGGER,
			"pipe: header_head updated to {} at {}", tip.last_block_h, tip.height
		);

		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

/// Utility function to handle forks. From the forked block, jump backward
/// to find to fork root. Rewind the txhashset to the root and apply all the
/// forked blocks prior to the one being processed to set the txhashset in
/// the expected state.
pub fn rewind_and_apply_fork(b: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	// extending a fork, first identify the block where forking occurred
	// keeping the hashes of blocks along the fork
	let mut current = b.header.previous;
	let mut fork_hashes = vec![];
	loop {
		let curr_header = ext.batch.get_block_header(&current)?;

		if let Ok(_) = ext.batch.is_on_current_chain(&curr_header) {
			break;
		} else {
			fork_hashes.insert(0, (curr_header.height, curr_header.hash()));
			current = curr_header.previous;
		}
	}

	let forked_header = ext.batch.get_block_header(&current)?;

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
		let fb = ext
			.batch
			.get_block(&h)
			.map_err(|e| ErrorKind::StoreErr(e, format!("getting forked blocks")))?;

		// Re-verify coinbase maturity along this fork.
		verify_coinbase_maturity(&fb, ext)?;
		// Validate the block against the UTXO set.
		validate_utxo(&fb, ext)?;
		// Re-verify block_sums to set the block_sums up on this fork correctly.
		verify_block_sums(&fb, ext)?;
		// Re-apply the blocks.
		apply_block_to_txhashset(&fb, ext)?;
	}
	Ok(())
}

fn validate_utxo(block: &Block, ext: &txhashset::Extension) -> Result<(), Error> {
	let utxo = ext.utxo_view();
	utxo.validate_block(block)?;
	Ok(())
}
