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
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader};
use core::global;
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

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain. Returns the new chain head if
/// updated.
pub fn process_block(b: &Block, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	debug!(
		LOGGER,
		"pipe: process_block {} at {} with {} inputs, {} outputs, {} kernels",
		b.hash(),
		b.header.height,
		b.inputs.len(),
		b.outputs.len(),
		b.kernels.len(),
	);
	check_known(b.hash(), ctx)?;

	validate_header(&b.header, ctx)?;

	// now check we actually have the previous block in the store
	// not just the header but the block itself
	// short circuit the test first both for performance (in-mem vs db access)
	// but also for the specific case of the first fast sync full block
	if b.header.previous != ctx.head.last_block_h {
		// we cannot assume we can use the chain head for this as we may be dealing
		// with a fork we cannot use heights here as the fork may have jumped in
		// height
		match ctx.store.block_exists(&b.header.previous) {
			Ok(true) => {}
			Ok(false) => {
				return Err(ErrorKind::Orphan.into());
			}
			Err(e) => {
				return Err(ErrorKind::StoreErr(e, "pipe get previous".to_owned()).into());
			}
		}
	}

	// validate the block itself
	// we can do this now before interacting with the txhashset
	let _sums = validate_block(b, ctx)?;

	// header and block both valid, and we have a previous block
	// so take the lock on the txhashset
	let local_txhashset = ctx.txhashset.clone();
	let mut txhashset = local_txhashset.write().unwrap();

	// update head now that we're in the lock
	ctx.head = ctx.store.head()?;

	let mut batch = ctx.store.batch()?;

	// start a chain extension unit of work dependent on the success of the
	// internal validation and saving operations
	txhashset::extending(&mut txhashset, &mut batch, |mut extension| {
		// First we rewind the txhashset extension if necessary
		// to put it into a consistent state for validating the block.
		// We can skip this step if the previous header is the latest header we saw.
		if b.header.previous != ctx.head.last_block_h {
			rewind_and_apply_fork(b, ctx.store.clone(), extension)?;
		}
		validate_block_via_txhashset(b, &mut extension)?;

		if !block_has_more_work(b, &ctx.head) {
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
	add_block(b, ctx.store.clone(), &mut batch)?;
	let res = update_head(b, &ctx, &mut batch);
	if res.is_ok() {
		batch.commit()?;
	}
	res
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

/// Quick in-memory check to fast-reject any block we've already handled
/// recently. Keeps duplicates from the network in check.
fn check_known(bh: Hash, ctx: &mut BlockContext) -> Result<(), Error> {
	if bh == ctx.head.last_block_h || bh == ctx.head.prev_block_h {
		return Err(ErrorKind::Unfit("already known".to_string()).into());
	}
	let cache = ctx.block_hashes_cache.read().unwrap();
	if cache.contains(&bh) {
		return Err(ErrorKind::Unfit("already known in cache".to_string()).into());
	}
	if ctx.orphans.contains(&bh) {
		return Err(ErrorKind::Unfit("already known in orphans".to_string()).into());
	}
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
		if global::min_sizeshift() > header.pow.cuckoo_sizeshift {
			return Err(ErrorKind::LowSizeshift.into());
		}
		if !(ctx.pow_verifier)(header, header.pow.cuckoo_sizeshift) {
			error!(
				LOGGER,
				"pipe: validate_header failed for cuckoo shift size {}",
				header.pow.cuckoo_sizeshift,
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
		if header.total_difficulty.clone() <= prev.total_difficulty.clone() {
			return Err(ErrorKind::DifficultyTooLow.into());
		}

		let target_difficulty = header.total_difficulty.clone() - prev.total_difficulty.clone();

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
				"validate_header: BANNABLE OFFENCE: header target difficulty {} != {}",
				target_difficulty.to_num(),
				network_difficulty.to_num()
			);
			return Err(ErrorKind::WrongTotalDifficulty.into());
		}
	}

	Ok(())
}

fn validate_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	if ctx.store.block_exists(&b.hash())? {
		if b.header.height < ctx.head.height.saturating_sub(50) {
			return Err(ErrorKind::OldBlock.into());
		} else {
			return Err(ErrorKind::Unfit("already known".to_string()).into());
		}
	}
	let prev = ctx.store.get_block_header(&b.header.previous)?;
	b.validate(&prev.total_kernel_offset, &prev.total_kernel_sum)
		.map_err(|e| ErrorKind::InvalidBlockProof(e))?;
	Ok(())
}

/// Fully validate the block by applying it to the txhashset extension
/// and checking the roots.
/// Rewind and reapply forked blocks if necessary to put the txhashset extension
/// in the correct state to accept the block.
fn validate_block_via_txhashset(b: &Block, ext: &mut txhashset::Extension) -> Result<(), Error> {
	// First check we are not attempting to spend any coinbase outputs
	// before they have matured sufficiently.
	ext.verify_coinbase_maturity(&b.inputs, b.header.height)?;

	// apply the new block to the MMR trees and check the new root hashes
	ext.apply_block(&b)?;

	let roots = ext.roots();
	if roots.output_root != b.header.output_root
		|| roots.rproof_root != b.header.range_proof_root
		|| roots.kernel_root != b.header.kernel_root
	{
		ext.dump(false);

		debug!(
			LOGGER,
			"validate_block_via_txhashset: output roots - {:?}, {:?}",
			roots.output_root,
			b.header.output_root,
		);
		debug!(
			LOGGER,
			"validate_block_via_txhashset: rproof roots - {:?}, {:?}",
			roots.rproof_root,
			b.header.range_proof_root,
		);
		debug!(
			LOGGER,
			"validate_block_via_txhashset: kernel roots - {:?}, {:?}",
			roots.kernel_root,
			b.header.kernel_root,
		);

		return Err(ErrorKind::InvalidRoot.into());
	}
	let sizes = ext.sizes();
	if b.header.output_mmr_size != sizes.0 || b.header.kernel_mmr_size != sizes.2 {
		return Err(ErrorKind::InvalidMMRSize.into());
	}

	Ok(())
}

/// Officially adds the block to our chain.
fn add_block(
	b: &Block,
	store: Arc<store::ChainStore>,
	batch: &mut store::Batch,
) -> Result<(), Error> {
	batch
		.save_block(b)
		.map_err(|e| ErrorKind::StoreErr(e, "pipe save block".to_owned()))?;
	let bitmap = store.build_and_cache_block_input_bitmap(&b)?;
	batch.save_block_input_bitmap(&b.hash(), &bitmap)?;
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
	if block_has_more_work(b, &ctx.head) {
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
fn block_has_more_work(b: &Block, tip: &Tip) -> bool {
	let block_tip = Tip::from_block(&b.header);
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

	let head_header = store.head_header()?;
	let forked_header = store.get_block_header(&current)?;

	trace!(
		LOGGER,
		"rewind_and_apply_fork @ {} [{}], was @ {} [{}]",
		forked_header.height,
		forked_header.hash(),
		b.header.height,
		b.header.hash()
	);

	// rewind the sum trees up to the forking block
	ext.rewind(&forked_header, &head_header)?;

	trace!(
		LOGGER,
		"rewind_and_apply_fork: blocks on fork: {:?}",
		fork_hashes,
	);

	// apply all forked blocks, including this new one
	for (_, h) in fork_hashes {
		let fb = store
			.get_block(&h)
			.map_err(|e| ErrorKind::StoreErr(e, format!("getting forked blocks")))?;
		ext.apply_block(&fb)?;
	}
	Ok(())
}
