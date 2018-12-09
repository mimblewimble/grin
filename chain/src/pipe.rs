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

use crate::chain::OrphanBlockPool;
use crate::core::consensus;
use crate::core::core::hash::Hashed;
use crate::core::core::verifier_cache::VerifierCache;
use crate::core::core::Committed;
use crate::core::core::{Block, BlockHeader, BlockSums};
use crate::core::global;
use crate::core::pow;
use crate::error::{Error, ErrorKind};
use crate::store;
use crate::txhashset;
use crate::types::{Options, Tip};
use crate::util::RwLock;
use chrono::prelude::Utc;
use chrono::Duration;
use grin_store;
use std::sync::Arc;

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext<'a> {
	/// The options
	pub opts: Options,
	/// The pow verifier to use when processing a block.
	pub pow_verifier: fn(&BlockHeader) -> Result<(), pow::Error>,
	/// The active txhashset (rewindable MMRs) to use for block processing.
	pub txhashset: &'a mut txhashset::TxHashSet,
	/// The active batch to use for block processing.
	pub batch: store::Batch<'a>,
	/// The verifier cache (caching verifier for rangeproofs and kernel signatures)
	pub verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	/// Recent orphan blocks to avoid double-processing
	pub orphans: Arc<OrphanBlockPool>,
}

/// Process a block header as part of processing a full block.
/// We want to be sure the header is valid before processing the full block.
fn process_header_for_block(
	header: &BlockHeader,
	is_fork: bool,
	ctx: &mut BlockContext<'_>,
) -> Result<(), Error> {
	txhashset::header_extending(&mut ctx.txhashset, &mut ctx.batch, |extension| {
		extension.force_rollback();
		if is_fork {
			rewind_and_apply_header_fork(header, extension)?;
		}
		extension.validate_root(header)?;
		extension.apply_header(header)?;
		Ok(())
	})?;

	validate_header(header, ctx)?;
	add_block_header(header, &ctx.batch)?;
	update_header_head(header, ctx)?;

	Ok(())
}

// Check if we already know about this block for various reasons
// from cheapest to most expensive (delay hitting the db until last).
fn check_known(block: &Block, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	check_known_head(&block.header, ctx)?;
	check_known_orphans(&block.header, ctx)?;
	check_known_store(&block.header, ctx)?;
	Ok(())
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain.
/// Returns new head if chain head updated.
pub fn process_block(b: &Block, ctx: &mut BlockContext<'_>) -> Result<Option<Tip>, Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	debug!(
		"pipe: process_block {} at {}, in/out/kern: {}/{}/{}",
		b.hash(),
		b.header.height,
		b.inputs().len(),
		b.outputs().len(),
		b.kernels().len(),
	);

	// Check if we have already processed this block previously.
	check_known(b, ctx)?;

	// Delay hitting the db for current chain head until we know
	// this block is not already known.
	let head = ctx.batch.head()?;
	let is_next = b.header.prev_hash == head.last_block_h;

	let prev = prev_header_store(&b.header, &mut ctx.batch)?;

	// Block is an orphan if we do not know about the previous full block.
	// Skip this check if we have just processed the previous block
	// or the full txhashset state (fast sync) at the previous block height.
	if !is_next && !ctx.batch.block_exists(&prev.hash())? {
		return Err(ErrorKind::Orphan.into());
	}

	// This is a fork in the context of both header and block processing
	// if this block does not immediately follow the chain head.
	let is_fork = !is_next;

	// Check the header is valid before we proceed with the full block.
	process_header_for_block(&b.header, is_fork, ctx)?;

	// Validate the block itself, make sure it is internally consistent.
	// Use the verifier_cache for verifying rangeproofs and kernel signatures.
	validate_block(b, ctx)?;

	// Start a chain extension unit of work dependent on the success of the
	// internal validation and saving operations
	txhashset::extending(&mut ctx.txhashset, &mut ctx.batch, |mut extension| {
		if is_fork {
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

	// Add the validated block to the db.
	// We do this even if we have not increased the total cumulative work
	// so we can maintain multiple (in progress) forks.
	add_block(b, &ctx.batch)?;

	if ctx.batch.tail().is_err() {
		update_body_tail(&b.header, &ctx.batch)?;
	}

	// Update the chain head if total work is increased.
	let res = update_head(b, ctx)?;
	Ok(res)
}

/// Process the block header.
/// This is only ever used during sync and uses a context based on sync_head.
pub fn sync_block_headers(
	headers: &[BlockHeader],
	ctx: &mut BlockContext<'_>,
) -> Result<Option<Tip>, Error> {
	if let Some(header) = headers.first() {
		debug!(
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
		let first_header = headers.first().unwrap();
		let prev_header = ctx.batch.get_previous_header(&first_header)?;
		txhashset::sync_extending(&mut ctx.txhashset, &mut ctx.batch, |extension| {
			extension.rewind(&prev_header)?;

			for header in headers {
				// Check the current root is correct.
				extension.validate_root(header)?;

				// Apply the header to the header MMR.
				extension.apply_header(header)?;

				// Save the header to the db.
				add_block_header(header, &extension.batch)?;
			}

			Ok(())
		})?;

		// Validate all our headers now that we have added each "previous"
		// header to the db in this batch above.
		for header in headers {
			validate_header(header, ctx)?;
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

/// Process block header as part of "header first" block propagation.
/// We validate the header but we do not store it or update header head based
/// on this. We will update these once we get the block back after requesting
/// it.
pub fn process_block_header(header: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	debug!(
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
fn check_header_known(header: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	let header_head = ctx.batch.header_head()?;
	if header.hash() == header_head.last_block_h || header.hash() == header_head.prev_block_h {
		return Err(ErrorKind::Unfit("header already known".to_string()).into());
	}
	Ok(())
}

/// Quick in-memory check to fast-reject any block handled recently.
/// Keeps duplicates from the network in check.
/// Checks against the last_block_h and prev_block_h of the chain head.
fn check_known_head(header: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	let head = ctx.batch.head()?;
	let bh = header.hash();
	if bh == head.last_block_h || bh == head.prev_block_h {
		return Err(ErrorKind::Unfit("already known in head".to_string()).into());
	}
	Ok(())
}

/// Check if this block is in the set of known orphans.
fn check_known_orphans(header: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	if ctx.orphans.contains(&header.hash()) {
		Err(ErrorKind::Unfit("already known in orphans".to_string()).into())
	} else {
		Ok(())
	}
}

// Check if this block is in the store already.
fn check_known_store(header: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
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

// Find the previous header from the store.
// Return an Orphan error if we cannot find the previous header.
fn prev_header_store(
	header: &BlockHeader,
	batch: &mut store::Batch<'_>,
) -> Result<BlockHeader, Error> {
	let prev = batch.get_previous_header(&header).map_err(|e| match e {
		grin_store::Error::NotFoundErr(_) => ErrorKind::Orphan,
		_ => ErrorKind::StoreErr(e, "check prev header".into()),
	})?;
	Ok(prev)
}

/// First level of block validation that only needs to act on the block header
/// to make it as cheap as possible. The different validations are also
/// arranged by order of cost to have as little DoS surface as possible.
fn validate_header(header: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	// check version, enforces scheduled hard fork
	if !consensus::valid_header_version(header.height, header.version) {
		error!(
			"Invalid block header version received ({}), maybe update Grin?",
			header.version
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
			return Err(ErrorKind::LowEdgebits.into());
		}
		let edge_bits = header.pow.edge_bits();
		if !(ctx.pow_verifier)(header).is_ok() {
			error!(
				"pipe: error validating header with cuckoo edge_bits {}",
				edge_bits
			);
			return Err(ErrorKind::InvalidPow.into());
		}
	}

	// First I/O cost, delayed as late as possible.
	let prev = prev_header_store(header, &mut ctx.batch)?;

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

		if header.pow.to_difficulty(header.height) < target_difficulty {
			return Err(ErrorKind::DifficultyTooLow.into());
		}

		// explicit check to ensure total_difficulty has increased by exactly
		// the _network_ difficulty of the previous block
		// (during testnet1 we use _block_ difficulty here)
		let child_batch = ctx.batch.child()?;
		let diff_iter = store::DifficultyIter::from_batch(prev.hash(), child_batch);
		let next_header_info = consensus::next_difficulty(header.height, diff_iter);
		if target_difficulty != next_header_info.difficulty {
			info!(
				"validate_header: header target difficulty {} != {}",
				target_difficulty.to_num(),
				next_header_info.difficulty.to_num()
			);
			return Err(ErrorKind::WrongTotalDifficulty.into());
		}
		// check the secondary PoW scaling factor if applicable
		if header.pow.secondary_scaling != next_header_info.secondary_scaling {
			info!(
				"validate_header: header secondary scaling {} != {}",
				header.pow.secondary_scaling, next_header_info.secondary_scaling
			);
			return Err(ErrorKind::InvalidScaling.into());
		}
	}

	Ok(())
}

fn validate_block(block: &Block, ctx: &mut BlockContext<'_>) -> Result<(), Error> {
	let prev = ctx.batch.get_previous_header(&block.header)?;
	block
		.validate(&prev.total_kernel_offset, ctx.verifier_cache.clone())
		.map_err(|e| ErrorKind::InvalidBlockProof(e))?;
	Ok(())
}

/// TODO - This can move into the utxo_view.
/// Verify the block is not attempting to spend coinbase outputs
/// before they have sufficiently matured.
/// Note: requires a txhashset extension.
fn verify_coinbase_maturity(
	block: &Block,
	ext: &mut txhashset::Extension<'_>,
) -> Result<(), Error> {
	ext.verify_coinbase_maturity(&block.inputs(), block.header.height)?;
	Ok(())
}

/// Some "real magick" verification logic.
/// The (BlockSums, Block) tuple implements Committed...
/// This allows us to verify kernel sums across the full utxo and kernel sets
/// based on block_sums of previous block, accounting for the inputs|outputs|kernels
/// of the new block.
fn verify_block_sums(b: &Block, ext: &mut txhashset::Extension<'_>) -> Result<(), Error> {
	// TODO - this is 2 db calls, can we optimize this?
	// Retrieve the block_sums for the previous block.
	let prev = ext.batch.get_previous_header(&b.header)?;
	let block_sums = ext.batch.get_block_sums(&prev.hash())?;

	// Overage is based purely on the new block.
	// Previous block_sums have taken all previous overage into account.
	let overage = b.header.overage();

	// Offset on the other hand is the total kernel offset from the new block.
	let offset = b.header.total_kernel_offset();

	// Verify the kernel sums for the block_sums with the new block applied.
	let (utxo_sum, kernel_sum) =
		(block_sums, b as &dyn Committed).verify_kernel_sums(overage, offset)?;

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
fn apply_block_to_txhashset(
	block: &Block,
	ext: &mut txhashset::Extension<'_>,
) -> Result<(), Error> {
	ext.validate_header_root(&block.header)?;
	ext.apply_block(block)?;
	ext.validate_roots()?;
	ext.validate_sizes()?;
	Ok(())
}

/// Officially adds the block to our chain.
/// Header must be added separately (assume this has been done previously).
fn add_block(b: &Block, batch: &store::Batch<'_>) -> Result<(), Error> {
	batch
		.save_block(b)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save block".to_owned()))?;
	Ok(())
}

/// Update the block chain tail so we can know the exact tail of full blocks in this node
fn update_body_tail(bh: &BlockHeader, batch: &store::Batch<'_>) -> Result<(), Error> {
	let tip = Tip::from_header(bh);
	batch
		.save_body_tail(&tip)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save body tail".to_owned()))?;
	debug!("body tail {} @ {}", bh.hash(), bh.height);
	Ok(())
}

/// Officially adds the block header to our header chain.
fn add_block_header(bh: &BlockHeader, batch: &store::Batch<'_>) -> Result<(), Error> {
	batch
		.save_block_header(bh)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save header".to_owned()))?;
	Ok(())
}

/// Directly updates the head if we've just appended a new block to it or handle
/// the situation where we've just added enough work to have a fork with more
/// work than the head.
fn update_head(b: &Block, ctx: &BlockContext<'_>) -> Result<Option<Tip>, Error> {
	// if we made a fork with more work than the head (which should also be true
	// when extending the head), update it
	let head = ctx.batch.head()?;
	if has_more_work(&b.header, &head) {
		let tip = Tip::from_header(&b.header);

		ctx.batch
			.save_body_head(&tip)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe save body".to_owned()))?;

		debug!(
			"pipe: head updated to {} at {}",
			tip.last_block_h, tip.height
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
fn update_sync_head(bh: &BlockHeader, batch: &mut store::Batch<'_>) -> Result<(), Error> {
	let tip = Tip::from_header(bh);
	batch
		.save_sync_head(&tip)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save sync head".to_owned()))?;
	debug!("sync head {} @ {}", bh.hash(), bh.height);
	Ok(())
}

/// Update the header head if this header has most work.
fn update_header_head(bh: &BlockHeader, ctx: &mut BlockContext<'_>) -> Result<Option<Tip>, Error> {
	let header_head = ctx.batch.header_head()?;
	if has_more_work(&bh, &header_head) {
		let tip = Tip::from_header(bh);
		ctx.batch
			.save_header_head(&tip)
			.map_err(|e| ErrorKind::StoreErr(e, "pipe save header head".to_owned()))?;

		debug!(
			"pipe: header_head updated to {} at {}",
			tip.last_block_h, tip.height
		);

		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

/// Rewind the header chain and reapply headers on a fork.
pub fn rewind_and_apply_header_fork(
	header: &BlockHeader,
	ext: &mut txhashset::HeaderExtension<'_>,
) -> Result<(), Error> {
	let mut fork_hashes = vec![];
	let mut current = ext.batch.get_previous_header(header)?;
	while current.height > 0 && !ext.is_on_current_chain(&current).is_ok() {
		fork_hashes.push(current.hash());
		current = ext.batch.get_previous_header(&current)?;
	}
	fork_hashes.reverse();

	let forked_header = current;

	// Rewind the txhashset state back to the block where we forked from the most work chain.
	ext.rewind(&forked_header)?;

	// Re-apply all headers on this fork.
	for h in fork_hashes {
		let header = ext
			.batch
			.get_block_header(&h)
			.map_err(|e| ErrorKind::StoreErr(e, format!("getting forked headers")))?;
		ext.apply_header(&header)?;
	}
	Ok(())
}

/// Utility function to handle forks. From the forked block, jump backward
/// to find to fork root. Rewind the txhashset to the root and apply all the
/// forked blocks prior to the one being processed to set the txhashset in
/// the expected state.
pub fn rewind_and_apply_fork(b: &Block, ext: &mut txhashset::Extension<'_>) -> Result<(), Error> {
	// extending a fork, first identify the block where forking occurred
	// keeping the hashes of blocks along the fork
	let mut fork_hashes = vec![];
	let mut current = ext.batch.get_previous_header(&b.header)?;
	while current.height > 0 && !ext.is_on_current_chain(&current).is_ok() {
		fork_hashes.push(current.hash());
		current = ext.batch.get_previous_header(&current)?;
	}
	fork_hashes.reverse();

	let forked_header = current;

	// Rewind the txhashset state back to the block where we forked from the most work chain.
	ext.rewind(&forked_header)?;

	// Now re-apply all blocks on this fork.
	for h in fork_hashes {
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

fn validate_utxo(block: &Block, ext: &txhashset::Extension<'_>) -> Result<(), Error> {
	let utxo = ext.utxo_view();
	utxo.validate_block(block)?;
	Ok(())
}
