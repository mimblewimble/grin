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

use std::convert::From;
use std::sync::{Arc, Mutex};

use secp;
use time;

use core::consensus;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::core::{BlockHeader, Block, Proof};
use core::pow;
use core::ser;
use types::*;
use store;

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext {
	pub opts: Options,
	pub store: Arc<ChainStore>,
	pub adapter: Arc<ChainAdapter>,
	pub head: Tip,
	pub lock: Arc<Mutex<bool>>,
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain. Returns the new
/// chain head if updated.
pub fn process_block(b: &Block, mut ctx: BlockContext) -> Result<Option<Tip>, Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	info!("Starting validation pipeline for block {} at {} with {} inputs and {} outputs.",
	      b.hash(),
	      b.header.height,
	      b.inputs.len(),
	      b.outputs.len());
	check_known(b.hash(), &mut ctx)?;

	if !ctx.opts.intersects(SYNC) {
		// in sync mode, the header has already been validated
		validate_header(&b.header, &mut ctx)?;
	}
	validate_block(b, &mut ctx)?;
	debug!("Block at {} with hash {} is valid, going to save and append.",
	       b.header.height,
	       b.hash());

	ctx.lock.lock();
	add_block(b, &mut ctx)?;
	update_head(b, &mut ctx)
}

pub fn process_block_header(bh: &BlockHeader, mut ctx: BlockContext) -> Result<Option<Tip>, Error> {

	info!("Starting validation pipeline for block header {} at {}.",
	      bh.hash(),
	      bh.height);
	check_known(bh.hash(), &mut ctx)?;
	validate_header(&bh, &mut ctx)?;
	add_block_header(bh, &mut ctx)?;

	ctx.lock.lock();
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

	let prev = try!(ctx.store.get_block_header(&header.previous).map_err(&Error::StoreErr));

	if header.height != prev.height + 1 {
		return Err(Error::InvalidBlockHeight);
	}
	if header.timestamp <= prev.timestamp {
		// prevent time warp attacks and some timestamp manipulations by forcing strict
		// time progression
		return Err(Error::InvalidBlockTime);
	}
	if header.timestamp >
	   time::now() + time::Duration::seconds(12 * (consensus::BLOCK_TIME_SEC as i64)) {
		// refuse blocks more than 12 blocks intervals in future (as in bitcoin)
		// TODO add warning in p2p code if local time is too different from peers
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

		let cycle_size = if ctx.opts.intersects(EASY_POW) {
			consensus::TEST_SIZESHIFT
		} else {
			consensus::DEFAULT_SIZESHIFT
		};
		debug!("Validating block with cuckoo size {}", cycle_size);
		if !pow::verify_size(header, cycle_size as u32) {
			return Err(Error::InvalidPow);
		}
	}

	Ok(())
}

/// Fully validate the block content.
fn validate_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	if b.header.height > ctx.head.height + 1 {
		// check orphan again, an orphan coming out of order from sync will have
		// bypassed header checks
		// TODO actually handle orphans and add them to a size-limited set
		return Err(Error::Unfit("orphan".to_string()));
	}

	let curve = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	try!(b.validate(&curve).map_err(&Error::InvalidBlockProof));

	// TODO check every input exists as a UTXO using the UTXO index

	Ok(())
}

/// Officially adds the block to our chain.
fn add_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.store.save_block(b).map_err(&Error::StoreErr)?;

	if !ctx.opts.intersects(SYNC) {
		// broadcast the block
		let adapter = ctx.adapter.clone();
		adapter.block_accepted(b);
	}
	Ok(())
}

/// Officially adds the block header to our header chain.
fn add_block_header(bh: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.store.save_block_header(bh).map_err(&Error::StoreErr)
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
		ctx.store.setup_height(&b.header).map_err(&Error::StoreErr)?;

		// in sync mode, only update the "body chain", otherwise update both the
		// "header chain" and "body chain"
		if ctx.opts.intersects(SYNC) {
			ctx.store.save_body_head(&tip).map_err(&Error::StoreErr)?;
		} else {
			ctx.store.save_head(&tip).map_err(&Error::StoreErr)?;
		}

		ctx.head = tip.clone();
		info!("Updated head to {} at {}.", b.hash(), b.header.height);
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
		ctx.store.save_header_head(&tip).map_err(&Error::StoreErr)?;

		ctx.head = tip.clone();
		info!("Updated block header head to {} at {}.",
		      bh.hash(),
		      bh.height);
		Ok(Some(tip))
	} else {
		Ok(None)
	}
}
