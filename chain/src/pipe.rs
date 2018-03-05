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

use time;

use core::consensus;
use core::core::hash::{Hash, Hashed};
use core::core::{Block, BlockHeader};
use core::core::target::Difficulty;
use grin_store;
use types::*;
use store;
use txhashset;
use core::global;
use util::LOGGER;

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext {
	/// The options
	pub opts: Options,
	/// The store
	pub store: Arc<ChainStore>,
	/// The head
	pub head: Tip,
	/// The POW verification function
	pub pow_verifier: fn(&BlockHeader, u32) -> bool,
	/// MMR sum tree states
	pub txhashset: Arc<RwLock<txhashset::TxHashSet>>,
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain. Returns the new
/// chain head if updated.
pub fn process_block(b: &Block, mut ctx: BlockContext) -> Result<Option<Tip>, Error> {
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
	check_known(b.hash(), &mut ctx)?;

	validate_header(&b.header, &mut ctx)?;

	// valid header, now check we actually have the previous block in the store
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
				return Err(Error::Orphan);
			}
			Err(e) => {
				return Err(Error::StoreErr(e, "pipe get previous".to_owned()));
			}
		}
	}

	// valid header and we have a previous block, time to take the lock on the sum
	// trees
	let local_txhashset = ctx.txhashset.clone();
	let mut txhashset = local_txhashset.write().unwrap();

	// update head now that we're in the lock
	ctx.head = ctx.store
		.head()
		.map_err(|e| Error::StoreErr(e, "pipe reload head".to_owned()))?;

	// start a chain extension unit of work dependent on the success of the
	// internal validation and saving operations
	let result = txhashset::extending(&mut txhashset, |mut extension| {
		validate_block(b, &mut ctx, &mut extension)?;
		debug!(
			LOGGER,
			"pipe: process_block: {} at {} is valid, save and append.",
			b.hash(),
			b.header.height,
		);

		add_block(b, &mut ctx)?;
		let h = update_head(b, &mut ctx)?;
		if h.is_none() {
			extension.force_rollback();
		}
		Ok(h)
	});

	match result {
		Ok(t) => {
			save_pmmr_metadata(&Tip::from_block(&b.header), &txhashset, ctx.store.clone())?;
			Ok(t)
		}
		Err(e) => Err(e),
	}
}

/// Save pmmr index location for a given block
pub fn save_pmmr_metadata(
	t: &Tip,
	txhashset: &txhashset::TxHashSet,
	store: Arc<ChainStore>,
) -> Result<(), Error> {
	// Save pmmr file metadata for this block
	let block_file_md = txhashset.last_file_metadata();
	store
		.save_block_pmmr_file_metadata(&t.last_block_h, &block_file_md)
		.map_err(|e| Error::StoreErr(e, "saving pmmr file metadata".to_owned()))?;
	Ok(())
}

/// Process the block header.
/// This is only ever used during sync and uses a context based on sync_head.
pub fn sync_block_header(
	bh: &BlockHeader,
	mut sync_ctx: BlockContext,
	mut header_ctx: BlockContext,
) -> Result<Option<Tip>, Error> {
	debug!(
		LOGGER,
		"pipe: sync_block_header: {} at {}",
		bh.hash(),
		bh.height
	);

	validate_header(&bh, &mut sync_ctx)?;
	add_block_header(bh, &mut sync_ctx)?;

	// TODO - confirm this is needed during sync process (I don't see how it is)
	// we do not touch the txhashset when syncing headers
	// just taking the shared lock
	let _ = header_ctx.txhashset.write().unwrap();

	// now update the header_head (if new header with most work) and the sync_head
	// (always)
	update_header_head(bh, &mut header_ctx)?;
	update_sync_head(bh, &mut sync_ctx)
}

/// Process block header as part of "header first" block propagation.
pub fn process_block_header(bh: &BlockHeader, mut ctx: BlockContext) -> Result<Option<Tip>, Error> {
	debug!(
		LOGGER,
		"pipe: process_block_header: {} at {}",
		bh.hash(),
		bh.height
	);

	check_header_known(bh.hash(), &mut ctx)?;
	validate_header(&bh, &mut ctx)?;

	debug!(
		LOGGER,
		"pipe: process_block_header: {} at {} is valid, saving.",
		bh.hash(),
		bh.height,
	);

	add_block_header(bh, &mut ctx)?;

	// now update the header_head (if new header with most work)
	update_header_head(bh, &mut ctx)
}

/// Quick in-memory check to fast-reject any block header we've already handled
/// recently. Keeps duplicates from the network in check.
/// ctx here is specific to the header_head (tip of the header chain)
fn check_header_known(bh: Hash, ctx: &mut BlockContext) -> Result<(), Error> {
	// TODO ring buffer of the last few blocks that came through here
	if bh == ctx.head.last_block_h || bh == ctx.head.prev_block_h {
		return Err(Error::Unfit("already known".to_string()));
	}
	if let Ok(h) = ctx.store.get_block_header(&bh) {
		// there is a window where a block header can be saved but the chain head not
		// updated yet, we plug that window here by re-accepting the block
		if h.total_difficulty <= ctx.head.total_difficulty {
			return Err(Error::Unfit("already in store".to_string()));
		}
	}
	Ok(())
}

/// Quick in-memory check to fast-reject any block we've already handled
/// recently. Keeps duplicates from the network in check.
fn check_known(bh: Hash, ctx: &mut BlockContext) -> Result<(), Error> {
	// TODO ring buffer of the last few blocks that came through here
	if bh == ctx.head.last_block_h || bh == ctx.head.prev_block_h {
		return Err(Error::Unfit("already known".to_string()));
	}
	if let Ok(b) = ctx.store.get_block(&bh) {
		// there is a window where a block can be saved but the chain head not
		// updated yet, we plug that window here by re-accepting the block
		if b.header.total_difficulty <= ctx.head.total_difficulty {
			return Err(Error::Unfit("already in store".to_string()));
		}
	}
	Ok(())
}

/// First level of block validation that only needs to act on the block header
/// to make it as cheap as possible. The different validations are also
/// arranged by order of cost to have as little DoS surface as possible.
/// TODO require only the block header (with length information)
fn validate_header(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	// check version, enforces scheduled hard fork
	if !consensus::valid_header_version(header.height, header.version) {
		error!(
			LOGGER,
			"Invalid block header version received ({}), maybe update Grin?", header.version
		);
		return Err(Error::InvalidBlockVersion(header.version));
	}

	if header.timestamp
		> time::now_utc() + time::Duration::seconds(12 * (consensus::BLOCK_TIME_SEC as i64))
	{
		// refuse blocks more than 12 blocks intervals in future (as in bitcoin)
		// TODO add warning in p2p code if local time is too different from peers
		return Err(Error::InvalidBlockTime);
	}

	if !ctx.opts.contains(Options::SKIP_POW) {
		let n = global::sizeshift() as u32;
		if !(ctx.pow_verifier)(header, n) {
			error!(
				LOGGER,
				"pipe: validate_header failed for cuckoo shift size {}", n
			);
			return Err(Error::InvalidPow);
		}
		if header.height % 500 == 0 {
			debug!(
				LOGGER,
				"Validating header validated, using cuckoo shift size {}", n
			);
		}
	}

	// first I/O cost, better as late as possible
	let prev = match ctx.store.get_block_header(&header.previous) {
		Ok(prev) => Ok(prev),
		Err(grin_store::Error::NotFoundErr) => Err(Error::Orphan),
		Err(e) => Err(Error::StoreErr(
			e,
			format!("previous header {}", header.previous),
		)),
	}?;

	if header.height != prev.height + 1 {
		return Err(Error::InvalidBlockHeight);
	}

	// TODO - get rid of the automated testing mode check here somehow
	if header.timestamp <= prev.timestamp && !global::is_automated_testing_mode() {
		// prevent time warp attacks and some timestamp manipulations by forcing strict
		// time progression (but not in CI mode)
		return Err(Error::InvalidBlockTime);
	}

	if !ctx.opts.contains(Options::SKIP_POW) {
		// verify the proof of work and related parameters

		// explicit check to ensure we are not below the minimum difficulty
		// we will also check difficulty based on next_difficulty later on
		if header.difficulty < Difficulty::one() {
			return Err(Error::DifficultyTooLow);
		}

		let diff_iter = store::DifficultyIter::from(header.previous, ctx.store.clone());
		let difficulty =
			consensus::next_difficulty(diff_iter).map_err(|e| Error::Other(e.to_string()))?;

		// explicit check to ensure total_difficulty has increased by exactly
		// the _network_ difficulty of the previous block
		// (during testnet1 we use _block_ difficulty here)
		if header.total_difficulty != prev.total_difficulty.clone() + difficulty.clone() {
			error!(
				LOGGER,
				"validate_header: BANNABLE OFFENCE: header cumulative difficulty {} != {}",
				header.difficulty.into_num(),
				prev.total_difficulty.into_num() + difficulty.into_num()
			);
			return Err(Error::WrongTotalDifficulty);
		}

		// now check that the difficulty is not less than that calculated by the
		// difficulty iterator based on the previous block
		if header.difficulty < difficulty {
			error!(
				LOGGER,
				"validate_header: BANNABLE OFFENCE: header difficulty {} < {}",
				header.difficulty.into_num(),
				difficulty.into_num()
			);
			return Err(Error::DifficultyTooLow);
		}
	}

	Ok(())
}

/// Fully validate the block content.
fn validate_block(
	b: &Block,
	ctx: &mut BlockContext,
	ext: &mut txhashset::Extension,
) -> Result<(), Error> {
	// main isolated block validation, checks all commitment sums and sigs
	b.validate().map_err(&Error::InvalidBlockProof)?;

	if b.header.previous != ctx.head.last_block_h {
		rewind_and_apply_fork(b, ctx.store.clone(), ext)?;
	}

	// apply the new block to the MMR trees and check the new root hashes
	ext.apply_block(&b)?;

	let roots = ext.roots();
	if roots.output_root != b.header.output_root || roots.rproof_root != b.header.range_proof_root
		|| roots.kernel_root != b.header.kernel_root
	{
		ext.dump(false);

		debug!(
			LOGGER,
			"validate_block: output roots - {:?}, {:?}", roots.output_root, b.header.output_root,
		);
		debug!(
			LOGGER,
			"validate_block: rproof roots - {:?}, {:?}",
			roots.rproof_root,
			b.header.range_proof_root,
		);
		debug!(
			LOGGER,
			"validate_block: kernel roots - {:?}, {:?}", roots.kernel_root, b.header.kernel_root,
		);

		return Err(Error::InvalidRoot);
	}

	Ok(())
}

/// Officially adds the block to our chain.
fn add_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.store
		.save_block(b)
		.map_err(|e| Error::StoreErr(e, "pipe save block".to_owned()))
}

/// Officially adds the block header to our header chain.
fn add_block_header(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.store
		.save_block_header(bh)
		.map_err(|e| Error::StoreErr(e, "pipe save header".to_owned()))
}

/// Directly updates the head if we've just appended a new block to it or handle
/// the situation where we've just added enough work to have a fork with more
/// work than the head.
fn update_head(b: &Block, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	// if we made a fork with more work than the head (which should also be true
	// when extending the head), update it
	let tip = Tip::from_block(&b.header);
	if tip.total_difficulty > ctx.head.total_difficulty {
		// update the block height index
		ctx.store
			.setup_height(&b.header, &ctx.head)
			.map_err(|e| Error::StoreErr(e, "pipe setup height".to_owned()))?;

		// in sync mode, only update the "body chain", otherwise update both the
		// "header chain" and "body chain", updating the header chain in sync resets
		// all additional "future" headers we've received
		if ctx.opts.contains(Options::SYNC) {
			ctx.store
				.save_body_head(&tip)
				.map_err(|e| Error::StoreErr(e, "pipe save body".to_owned()))?;
		} else {
			ctx.store
				.save_head(&tip)
				.map_err(|e| Error::StoreErr(e, "pipe save head".to_owned()))?;
		}
		ctx.head = tip.clone();
		if b.header.height % 100 == 0 {
			info!(
				LOGGER,
				"pipe: chain head reached {} @ {} [{}]",
				b.header.height,
				b.header.difficulty,
				b.hash()
			);
		} else {
			debug!(
				LOGGER,
				"pipe: chain head reached {} @ {} [{}]",
				b.header.height,
				b.header.difficulty,
				b.hash()
			);
		}
		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

/// Update the sync head so we can keep syncing from where we left off.
fn update_sync_head(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	let tip = Tip::from_block(bh);
	ctx.store
		.save_sync_head(&tip)
		.map_err(|e| Error::StoreErr(e, "pipe save sync head".to_owned()))?;
	ctx.head = tip.clone();
	if bh.height % 100 == 0 {
		info!(
			LOGGER,
			"sync head {} @ {} [{}]",
			bh.total_difficulty,
			bh.height,
			bh.hash()
		);
	} else {
		debug!(
			LOGGER,
			"sync head {} @ {} [{}]",
			bh.total_difficulty,
			bh.height,
			bh.hash()
		);
	}
	Ok(Some(tip))
}

fn update_header_head(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	let tip = Tip::from_block(bh);
	if tip.total_difficulty > ctx.head.total_difficulty {
		ctx.store
			.save_header_head(&tip)
			.map_err(|e| Error::StoreErr(e, "pipe save header head".to_owned()))?;
		ctx.head = tip.clone();
		if bh.height % 100 == 0 {
			info!(
				LOGGER,
				"header head {} @ {} [{}]",
				bh.total_difficulty,
				bh.height,
				bh.hash()
			);
		} else {
			debug!(
				LOGGER,
				"header head {} @ {} [{}]",
				bh.total_difficulty,
				bh.height,
				bh.hash()
			);
		}
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
	store: Arc<ChainStore>,
	ext: &mut txhashset::Extension,
) -> Result<(), Error> {
	// extending a fork, first identify the block where forking occurred
	// keeping the hashes of blocks along the fork
	let mut current = b.header.previous;
	let mut hashes = vec![];
	loop {
		let curr_header = store.get_block_header(&current)?;

		if let Ok(_) = store.is_on_current_chain(&curr_header) {
			break;
		} else {
			hashes.insert(0, curr_header.hash());
			current = curr_header.previous;
		}
	}

	let forked_block = store.get_block(&current)?;

	debug!(
		LOGGER,
		"rewind_and_apply_fork @ {} [{}]",
		forked_block.header.height,
		forked_block.header.hash(),
	);

	// rewind the sum trees up to the forking block
	ext.rewind(&forked_block)?;

	// apply all forked blocks, including this new one
	for h in hashes {
		let fb = store
			.get_block(&h)
			.map_err(|e| Error::StoreErr(e, format!("getting forked blocks")))?;
		ext.apply_block(&fb)?;
	}
	Ok(())
}
