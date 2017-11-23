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
use core::core::target::Difficulty;
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

	debug!(
		LOGGER,
		"pipe: process_block: {}, {}: {} inputs, {} outputs.",
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
			"pipe: process_block: {}, {}: valid, save and append.",
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
	debug!(
		LOGGER,
		"pipe: process_header: {}, {}.",
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
		let cycle_size = global::sizeshift();

		debug!(LOGGER, "Validating block header with cuckoo size {}", cycle_size);
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

		// explicit check to ensure we are not below the minimum difficulty
		// we will also check difficulty based on next_difficulty later on
		if header.difficulty < Difficulty::minimum() {
			return Err(Error::DifficultyTooLow);
		}

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

		let forked_block = ctx.store.get_block(&current)?;

		debug!(
			LOGGER,
			"pipe: validate_block: forked_block: {}, {}",
			forked_block.header.height,
			forked_block.header.hash(),
		);

		// rewind the sum trees up to the forking block
		ext.rewind(&forked_block)?;

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

		debug!(
			LOGGER,
			"validate_block: utxo roots - {:?}, {:?}",
			utxo_root.hash,
			b.header.utxo_root,
		);
		debug!(
			LOGGER,
			"validate_block: rproof roots - {:?}, {:?}",
			rproof_root.hash,
			b.header.range_proof_root,
		);
		debug!(
			LOGGER,
			"validate_block: kernel roots - {:?}, {:?}",
			kernel_root.hash,
			b.header.kernel_root,
		);

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
/// the situation where we have added enough work to have a fork with more
/// work than the head.
fn update_head(b: &Block, ctx: &mut BlockContext) -> Result<Option<Tip>, Error> {
	// We got this far, 3 scenarios -
	// 1) block chain and header chain are consistent
	//    and we are simply extending it with more work, so update both heads and setup height
	// 2) block chain and header chain are consistent, but
	//    this new block is a fork (with more work), so update both heads and setup height
	// 3) block chain and header chain are not consistent
	//    we are syncing with a peer that had advertised more work,
	//    we have (some of) the new header chain and do not want to update
	//    the head of the header chain (or heights) until we are caught up

	let tip = Tip::from_block(&b.header);

	let block_chain_tip: Tip = ctx.store.head()?;
	let header_chain_tip: Tip = ctx.store.get_header_head()?;

	// compare the heads of the two chains to see if they are consistent
	// we may have already received the latest header via sync so account for that
	let consistent_chains = block_chain_tip.last_block_h == header_chain_tip.last_block_h
		|| block_chain_tip.last_block_h == header_chain_tip.prev_block_h;

	// this block increases overall work if it is greater than current head of block chain
	let more_work = tip.total_difficulty > block_chain_tip.total_difficulty;

	let msg = format!(
		"tip - {}, {}, block  - {}, {}, header - {}, {}",
		tip.height,
		tip.last_block_h,
		block_chain_tip.height,
		block_chain_tip.last_block_h,
		header_chain_tip.height,
		header_chain_tip.last_block_h,
	);

	if more_work {
		if consistent_chains {
			// either we are simply extending the chain with a new block here
			// or we want to switch immediately to a fork with greater work

			debug!(LOGGER, "pipe: update_head:  >>>>: {}", msg);

			ctx.store
				.save_head(&tip)
				.map_err(|e| Error::StoreErr(e, "pipe save head".to_owned()))?;
		} else {
			// we are catching up (syncing) to the header chain
			// so update the body head (but not the header head)
			// even if the block chain has more work than the header chain
			// as we may not have received all headers yet from most_work_peer

			debug!(LOGGER, "pipe: update_head:  sync: {}", msg);

			ctx.store
				.save_body_head(&tip)
				.map_err(|e| Error::StoreErr(e, "pipe save body".to_owned()))?;
		}

		// TODO - do we always want to setup heights here?
		ctx.store
			.setup_height(&b.header)
			.map_err(|e| Error::StoreErr(e, "pipe setup height".to_owned()))?;

		ctx.head = tip.clone();

		info!(
			LOGGER,
			"Updated head to {} at {}.",
			tip.last_block_h,
			tip.height,
		);
		Ok(Some(tip))

	} else {
		// work did not increase, don't update heads or heights (probably never happens)
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

	let msg = format!(
		"tip - {}, {}, header - {}, {}",
		tip.height,
		tip.last_block_h,
		ctx.head.height,
		ctx.head.last_block_h,
	);

	debug!(LOGGER, "pipe: update_header_head: {}", msg);


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
