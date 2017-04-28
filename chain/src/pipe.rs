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
use grin_store;
use types;
use types::{Tip, ChainStore, ChainAdapter, NoopAdapter};
use store;

bitflags! {
  /// Options for block validation
  pub flags Options: u32 {
    const NONE = 0b00000001,
    /// Runs without checking the Proof of Work, mostly to make testing easier.
    const SKIP_POW = 0b00000010,
    /// Adds block while in syncing mode.
    const SYNC = 0b00000100,
  }
}

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext {
	opts: Options,
	store: Arc<ChainStore>,
	adapter: Arc<ChainAdapter>,
	head: Tip,
}

#[derive(Debug)]
pub enum Error {
	/// The block doesn't fit anywhere in our chain
	Unfit(String),
	/// Difficulty is too low either compared to ours or the block PoW hash
	DifficultyTooLow,
	/// Addition of difficulties on all previous block is wrong
	WrongTotalDifficulty,
	/// Size of the Cuckoo graph in block header doesn't match PoW requirements
	WrongCuckooSize,
	/// The proof of work is invalid
	InvalidPow,
	/// The block doesn't sum correctly or a tx signature is invalid
	InvalidBlockProof(secp::Error),
	/// Block time is too old
	InvalidBlockTime,
	/// Block height is invalid (not previous + 1)
	InvalidBlockHeight,
	/// Internal issue when trying to save or load data from store
	StoreErr(grin_store::Error),
	SerErr(ser::Error),
}

impl From<grin_store::Error> for Error {
	fn from(e: grin_store::Error) -> Error {
		Error::StoreErr(e)
	}
}
impl From<ser::Error> for Error {
	fn from(e: ser::Error) -> Error {
		Error::SerErr(e)
	}
}

/// Runs the block processing pipeline, including validation and finding a
/// place for the new block in the chain. Returns the new
/// chain head if updated.
pub fn process_block(b: &Block,
                     store: Arc<ChainStore>,
                     adapter: Arc<ChainAdapter>,
                     opts: Options)
                     -> Result<Option<Tip>, Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	let head = store.head().map_err(&Error::StoreErr)?;

	let mut ctx = BlockContext {
		opts: opts,
		store: store,
		adapter: adapter,
		head: head,
	};

	info!("Starting validation pipeline for block {} at {}.",
	      b.hash(),
	      b.header.height);
	try!(check_known(b.hash(), &mut ctx));

	if !ctx.opts.intersects(SYNC) {
		// in sync mode, the header has already been validated
		try!(validate_header(&b.header, &mut ctx));
	}
	try!(validate_block(b, &mut ctx));
	debug!("Block at {} with hash {} is valid, going to save and append.",
	       b.header.height,
	       b.hash());
	try!(add_block(b, &mut ctx));
	// TODO a global lock should be set before that step or even earlier
	update_head(b, &mut ctx)
}

pub fn process_block_header(bh: &BlockHeader,
                            store: Arc<ChainStore>,
                            adapter: Arc<ChainAdapter>,
                            opts: Options)
                            -> Result<Option<Tip>, Error> {

	let head = store.get_header_head().map_err(&Error::StoreErr)?;

	let mut ctx = BlockContext {
		opts: opts,
		store: store,
		adapter: adapter,
		head: head,
	};

	info!("Starting validation pipeline for block header {} at {}.",
	      bh.hash(),
	      bh.height);
	try!(check_known(bh.hash(), &mut ctx));
	try!(validate_header(&bh, &mut ctx));
	try!(add_block_header(bh, &mut ctx));
	// TODO a global lock should be set before that step or even earlier
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
		// TODO actually handle orphans and add them to a size-limited set
		return Err(Error::Unfit("orphan".to_string()));
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

		let (difficulty, cuckoo_sz) = consensus::next_target(header.timestamp.to_timespec().sec,
		                                                     prev.timestamp.to_timespec().sec,
		                                                     prev.difficulty,
		                                                     prev.cuckoo_len);
		if header.difficulty < difficulty {
			return Err(Error::DifficultyTooLow);
		}
		if header.cuckoo_len != cuckoo_sz {
			return Err(Error::WrongCuckooSize);
		}
		if !pow::verify(header) {
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
