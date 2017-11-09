// Copyright 2016 The Grin Developers
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
use core::core::transaction;
use types::*;
use store;
use sumtree;
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
	pub sumtrees: Arc<RwLock<sumtree::SumTrees>>,
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain. Returns the new
/// chain head if updated.
pub fn process_block(b: &Block, mut ctx: BlockContext) -> Result<Option<Tip>, Error> {
	// TODO should just take a promise for a block with a full header so we don't
 // spend resources reading the full block when its header is invalid

	info!(
		LOGGER,
		"Starting validation pipeline for block {} at {} with {} inputs and {} outputs.",
		b.hash(),
		b.header.height,
		b.inputs.len(),
		b.outputs.len()
	);
	check_known(b.hash(), &mut ctx)?;

	validate_header(&b.header, &mut ctx)?;

	// valid header, time to take the lock on the sum trees
	let local_sumtrees = ctx.sumtrees.clone();
	let mut sumtrees = local_sumtrees.write().unwrap();

	// update head now that we're in the lock
	ctx.head = ctx.store
		.head()
		.map_err(|e| Error::StoreErr(e, "pipe reload head".to_owned()))?;

	// start a chain extension unit of work dependent on the success of the
 // internal validation and saving operations
	sumtree::extending(&mut sumtrees, |mut extension| {
		validate_block(b, &mut ctx, &mut extension)?;
		debug!(
			LOGGER,
			"Block at {} with hash {} is valid, going to save and append.",
			b.header.height,
			b.hash()
		);

		add_block(b, &mut ctx)?;
		let h = update_head(b, &mut ctx)?;
		if h.is_none() {
			extension.force_rollback();
		}
		Ok(h)
	})
}

/// Process the block header
pub fn process_block_header(bh: &BlockHeader, mut ctx: BlockContext) -> Result<Option<Tip>, Error> {
	info!(
		LOGGER,
		"Starting validation pipeline for block header {} at {}.",
		bh.hash(),
		bh.height
	);
	check_known(bh.hash(), &mut ctx)?;
	validate_header(&bh, &mut ctx)?;
	add_block_header(bh, &mut ctx)?;

	// just taking the shared lock
	let _ = ctx.sumtrees.write().unwrap();

	update_header_head(bh, &mut ctx)
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

/// First level of black validation that only needs to act on the block header
/// to make it as cheap as possible. The different validations are also
/// arranged by order of cost to have as little DoS surface as possible.
/// TODO require only the block header (with length information)
fn validate_header(header: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	if header.height > ctx.head.height + 1 {
		return Err(Error::Orphan);
	}

	// check version, enforces scheduled hard fork
	if !consensus::valid_header_version(header.height, header.version) {
		error!(
			LOGGER,
			"Invalid block header version received ({}), maybe update Grin?",
			header.version
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

	if !ctx.opts.intersects(SKIP_POW) {
		let cycle_size = if ctx.opts.intersects(EASY_POW) {
			global::sizeshift()
		} else {
			consensus::DEFAULT_SIZESHIFT
		};
		debug!(LOGGER, "Validating block with cuckoo size {}", cycle_size);
		if !(ctx.pow_verifier)(header, cycle_size as u32) {
			return Err(Error::InvalidPow);
		}
	}

	// first I/O cost, better as late as possible
	let prev = try!(ctx.store.get_block_header(&header.previous,).map_err(|e| {
		Error::StoreErr(e, format!("previous block header {}", header.previous))
	},));

	if header.height != prev.height + 1 {
		return Err(Error::InvalidBlockHeight);
	}
	if header.timestamp <= prev.timestamp && !global::is_automated_testing_mode() {
		// prevent time warp attacks and some timestamp manipulations by forcing strict
  // time progression (but not in CI mode)
		return Err(Error::InvalidBlockTime);
	}

	if !ctx.opts.intersects(SKIP_POW) {
		// verify the proof of work and related parameters

		if header.total_difficulty != prev.total_difficulty.clone() + prev.pow.to_difficulty() {
			return Err(Error::WrongTotalDifficulty);
		}

		let diff_iter = store::DifficultyIter::from(header.previous, ctx.store.clone());
		let difficulty =
			consensus::next_difficulty(diff_iter).map_err(|e| Error::Other(e.to_string()))?;
		if header.difficulty < difficulty {
			return Err(Error::DifficultyTooLow);
		}
	}

	Ok(())
}

/// Fully validate the block content.
fn validate_block(
	b: &Block,
	ctx: &mut BlockContext,
	ext: &mut sumtree::Extension,
) -> Result<(), Error> {
	if b.header.height > ctx.head.height + 1 {
		return Err(Error::Orphan);
	}

	// main isolated block validation, checks all commitment sums and sigs
	try!(b.validate().map_err(&Error::InvalidBlockProof));

	// apply the new block to the MMR trees and check the new root hashes
	if b.header.previous == ctx.head.last_block_h {
		// standard head extension
		ext.apply_block(b)?;
	} else {
		// extending a fork, first identify the block where forking occurred
  // keeping the hashes of blocks along the fork
		let mut current = b.header.previous;
		let mut hashes = vec![];
		loop {
			let curr_header = ctx.store.get_block_header(&current)?;
			let height_header = ctx.store.get_header_by_height(curr_header.height)?;
			if curr_header.hash() != height_header.hash() {
				hashes.insert(0, curr_header.hash());
				current = curr_header.previous;
			} else {
				break;
			}
		}

		// rewind the sum trees up the forking block, providing the height of the
  // forked block and the last commitment we want to rewind to
		let forked_block = ctx.store.get_block(&current)?;
		if forked_block.header.height > 0 {
			let last_output = &forked_block.outputs[forked_block.outputs.len() - 1];
			let last_kernel = &forked_block.kernels[forked_block.kernels.len() - 1];
			ext.rewind(forked_block.header.height, last_output, last_kernel)?;
		}

		// apply all forked blocks, including this new one
		for h in hashes {
			let fb = ctx.store.get_block(&h)?;
			ext.apply_block(&fb)?;
		}
		ext.apply_block(&b)?;
	}

	let (utxo_root, rproof_root, kernel_root) = ext.roots();
	if utxo_root.hash != b.header.utxo_root || rproof_root.hash != b.header.range_proof_root
		|| kernel_root.hash != b.header.kernel_root
	{
		ext.dump(false);
		return Err(Error::InvalidRoot);
	}

	// check for any outputs with lock_heights greater than current block height
	for input in &b.inputs {
		if let Ok(output) = ctx.store.get_output_by_commit(&input.commitment()) {
			if output.features.contains(transaction::COINBASE_OUTPUT) {
				if let Ok(output_header) = ctx.store
					.get_block_header_by_output_commit(&input.commitment())
				{
					// TODO - make sure we are not off-by-1 here vs. the equivalent tansaction
	 // validation rule
					if b.header.height <= output_header.height + global::coinbase_maturity() {
						return Err(Error::ImmatureCoinbase);
					}
				};
			};
		};
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
			.setup_height(&b.header)
			.map_err(|e| Error::StoreErr(e, "pipe setup height".to_owned()))?;

		// in sync mode, only update the "body chain", otherwise update both the
  // "header chain" and "body chain", updating the header chain in sync resets
  // all additional "future" headers we've received
		if ctx.opts.intersects(SYNC) {
			ctx.store
				.save_body_head(&tip)
				.map_err(|e| Error::StoreErr(e, "pipe save body".to_owned()))?;
		} else {
			ctx.store
				.save_head(&tip)
				.map_err(|e| Error::StoreErr(e, "pipe save head".to_owned()))?;
		}
		ctx.head = tip.clone();
		info!(
			LOGGER,
			"Updated head to {} at {}.",
			b.hash(),
			b.header.height
		);
		Ok(Some(tip))
	} else {
		Ok(None)
	}
}

/// Directly updates the head if we've just appended a new block to it or handle
/// the situation where we've just added enough work to have a fork with more
/// work than the head.
fn update_header_head(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	// if we made a fork with more work than the head (which should also be true
 // when extending the head), update it
	let tip = Tip::from_block(bh);
	if tip.total_difficulty > ctx.head.total_difficulty {
		ctx.store
			.save_header_head(&tip)
			.map_err(|e| Error::StoreErr(e, "pipe save header head".to_owned()))?;

		ctx.head = tip.clone();
		info!(
			LOGGER,
			"Updated block header head to {} at {}.",
			bh.hash(),
			bh.height
		);
		Ok(Some(tip))
	} else {
		Ok(None)
	}
}
