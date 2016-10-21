//! Implementation of the chain block acceptance (or refusal) pipeline.

use secp;

use core::core::{Hash, BlockHeader, Block, Proof};
use core::pow;
use types;
use types::{Tip, ChainStore};
use store;

bitflags! {
  /// Options for block validation
  pub flags Options: u32 {
    /// Runs with the easier version of the Proof of Work, mostly to make testing easier.
    const EASY_POW = 0b00000001,
  }
}

/// Contextual information required to process a new block and either reject or
/// accept it.
pub struct BlockContext<'a> {
	opts: Options,
	store: &'a ChainStore,
	head: Tip,
	tip: Option<Tip>,
}

#[derive(Debug)]
pub enum Error {
	/// The block doesn't fit anywhere in our chain
	Unfit(String),
	/// The proof of work is invalid
	InvalidPow,
	/// The block doesn't sum correctly or a tx signature is invalid
	InvalidBlockProof(secp::Error),
	/// Internal issue when trying to save the block
	StoreErr(types::Error),
}

pub fn process_block(b: &Block, store: &ChainStore, opts: Options) -> Option<Error> {
	// TODO should just take a promise for a block with a full header so we don't
	// spend resources reading the full block when its header is invalid

	let head = match store.head() {
		Ok(head) => head,
		Err(err) => return Some(Error::StoreErr(err)),
	};
	let mut ctx = BlockContext {
		opts: opts,
		store: store,
		head: head,
		tip: None,
	};

	try_m!(validate_header(&b, &mut ctx));
	try_m!(set_tip(&b.header, &mut ctx));
	try_m!(validate_block(b, &mut ctx));
	try_m!(add_block(b, &mut ctx));
	try_m!(update_tips(&mut ctx));
	None
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
/// TODO actually require only the block header (with length information)
fn validate_header(b: &Block, ctx: &mut BlockContext) -> Option<Error> {
	let header = &b.header;
	println!("{} {}", header.height, ctx.head.height);
	if header.height > ctx.head.height + 1 {
		// TODO actually handle orphans and add them to a size-limited set
		return Some(Error::Unfit("orphan".to_string()));
	}
	// TODO check time wrt to chain time, refuse older than 100 blocks or too far
	// in future

	// TODO maintain current difficulty
	let diff_target = Proof(pow::MAX_TARGET);

	if ctx.opts.intersects(EASY_POW) {
		if !pow::verify20(b, diff_target) {
			return Some(Error::InvalidPow);
		}
	} else if !pow::verify(b, diff_target) {
		return Some(Error::InvalidPow);
	}
	None
}

fn set_tip(h: &BlockHeader, ctx: &mut BlockContext) -> Option<Error> {
	ctx.tip = Some(ctx.head.clone());
	None
}

fn validate_block(b: &Block, ctx: &mut BlockContext) -> Option<Error> {
	// TODO check tx merkle tree
	let curve = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	try_m!(b.verify(&curve).err().map(&Error::InvalidBlockProof));
	None
}

fn add_block(b: &Block, ctx: &mut BlockContext) -> Option<Error> {
	ctx.tip = ctx.tip.as_ref().map(|t| t.append(b.hash()));
	ctx.store.save_block(b).map(&Error::StoreErr)
}

fn update_tips(ctx: &mut BlockContext) -> Option<Error> {
	ctx.store.save_head(ctx.tip.as_ref().unwrap()).map(&Error::StoreErr)
}
