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

use std::sync::{Arc, Mutex};

use secp;
use time;

use core::consensus;
use core::core::hash::Hash;
use core::core::{BlockHeader, Block, Proof};
use core::pow;
use types;
use types::{Tip, ChainStore};
use store;

bitflags! {
  /// Options for block validation
  pub flags Options: u32 {
    const NONE = 0b00000001,
    /// Runs with the easier version of the Proof of Work, mostly to make testing easier.
    const EASY_POW = 0b00000010,
  }
}

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext {
	opts: Options,
	store: Arc<ChainStore>,
	head: Tip,
	tip: Option<Tip>,
}

#[derive(Debug)]
pub enum Error {
	/// The block doesn't fit anywhere in our chain
	Unfit(String),
	/// Target is too high either compared to ours or the block PoW hash
	TargetTooHigh,
	/// The proof of work is invalid
	InvalidPow,
	/// The block doesn't sum correctly or a tx signature is invalid
	InvalidBlockProof(secp::Error),
	/// Block time is too old
	InvalidBlockTime,
	/// Internal issue when trying to save or load data from store
	StoreErr(types::Error),
}

pub fn process_block(b: &Block, store: Arc<ChainStore>, opts: Options) -> Result<(), Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	let head = try!(store.head().map_err(&Error::StoreErr));
	let mut ctx = BlockContext {
		opts: opts,
		store: store,
		head: head,
		tip: None,
	};

	try!(validate_header(&b, &mut ctx));
	try!(set_tip(&b.header, &mut ctx));
	try!(validate_block(b, &mut ctx));
	try!(add_block(b, &mut ctx));
	try!(update_tips(&mut ctx));
	Ok(())
}

// block processing pipeline
// 1. is the header valid (time, PoW, etc.)
// 2. is it the next head, a new fork, or extension of a fork (not a too old
// fork tho)
// 3. ok fine, is all of it valid (txs, merkle, utxo merkle, etc.)
// 4. add the sucker to the head/fork
// 5. did we increase a fork difficulty over the head?
// 6. ok fine, swap them up (can be tricky, think addresses invalidation)

/// First level of black validation that only needs to act on the block header
/// to make it as cheap as possible. The different validations are also
/// arranged by order of cost to have as little DoS surface as possible.
/// TODO require only the block header (with length information)
fn validate_header(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	let header = &b.header;
	if header.height > ctx.head.height + 1 {
		// TODO actually handle orphans and add them to a size-limited set
		return Err(Error::Unfit("orphan".to_string()));
	}

	let prev = try!(ctx.store.get_block_header(&header.previous).map_err(&Error::StoreErr));

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

	let (diff_target, cuckoo_sz) = consensus::next_target(header.timestamp.to_timespec().sec,
	                                                      prev.timestamp.to_timespec().sec,
	                                                      prev.target,
	                                                      prev.cuckoo_len);
	if header.target > diff_target {
		return Err(Error::TargetTooHigh);
	}

	if ctx.opts.intersects(EASY_POW) {
		if !pow::verify_size(b, 15) {
			return Err(Error::InvalidPow);
		}
	} else if !pow::verify_size(b, cuckoo_sz as u32) {
		return Err(Error::InvalidPow);
	}

	Ok(())
}

fn set_tip(h: &BlockHeader, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.tip = Some(ctx.head.clone());
	Ok(())
}

fn validate_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	// TODO check tx merkle tree
	let curve = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	try!(b.verify(&curve).map_err(&Error::InvalidBlockProof));
	Ok(())
}

fn add_block(b: &Block, ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.tip = ctx.tip.as_ref().map(|t| t.append(b.hash()));
	ctx.store.save_block(b).map_err(&Error::StoreErr)
}

fn update_tips(ctx: &mut BlockContext) -> Result<(), Error> {
	ctx.store.save_head(ctx.tip.as_ref().unwrap()).map_err(&Error::StoreErr)
}
