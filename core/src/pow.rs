// Copyright 2021 The Grin Developers
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

//! The proof of work needs to strike a balance between fast header
//! verification to avoid DoS attacks and difficulty for block verifiers to
//! build new blocks. In addition, mining new blocks should also be as
//! difficult on high end custom-made hardware (ASICs) as on commodity hardware
//! or smartphones. For this reason we use Cuckoo Cycle (see the cuckoo
//! module for more information).
//!
//! Note that this miner implementation is here mostly for tests and
//! reference. It's not optimized for speed.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

pub use self::common::EdgeType;
pub use self::types::*;
use crate::core::{Block, BlockHeader};
use crate::genesis;
use crate::global;

#[macro_use]
mod common;
pub mod cuckaroo;
pub mod cuckarood;
pub mod cuckaroom;
pub mod cuckarooz;
pub mod cuckatoo;
mod error;
#[allow(dead_code)]
pub mod lean;
mod siphash;
mod types;

pub use crate::pow::cuckaroo::{new_cuckaroo_ctx, no_cuckaroo_ctx, CuckarooContext};
pub use crate::pow::cuckarood::{new_cuckarood_ctx, CuckaroodContext};
pub use crate::pow::cuckaroom::{new_cuckaroom_ctx, CuckaroomContext};
pub use crate::pow::cuckarooz::{new_cuckarooz_ctx, CuckaroozContext};
pub use crate::pow::cuckatoo::{new_cuckatoo_ctx, CuckatooContext};
pub use crate::pow::error::Error;
use chrono::prelude::{DateTime, NaiveDateTime, Utc};

const MAX_SOLS: u32 = 10;

/// Validates the proof of work of a given header, and that the proof of work
/// satisfies the requirements of the header.
pub fn verify_size(bh: &BlockHeader) -> Result<(), Error> {
	let mut ctx = global::create_pow_context::<u64>(
		bh.height,
		bh.pow.edge_bits(),
		bh.pow.proof.nonces.len(),
		MAX_SOLS,
	)?;
	ctx.set_header_nonce(bh.pre_pow(), None, false)?;
	ctx.verify(&bh.pow.proof)
}

/// Mines a genesis block using the internal miner
pub fn mine_genesis_block() -> Result<Block, Error> {
	let mut gen = genesis::genesis_dev();

	// total_difficulty on the genesis header *is* the difficulty of that block
	let genesis_difficulty = gen.header.pow.total_difficulty;

	let sz = global::min_edge_bits();
	let proof_size = global::proofsize();

	pow_size(&mut gen.header, genesis_difficulty, proof_size, sz)?;
	Ok(gen)
}

/// Runs a proof of work computation over the provided block using the provided
/// Mining Worker, until the required difficulty target is reached. May take a
/// while for a low target...
pub fn pow_size(
	bh: &mut BlockHeader,
	diff: Difficulty,
	proof_size: usize,
	sz: u8,
) -> Result<(), Error> {
	let start_nonce = bh.pow.nonce;

	// try to find a cuckoo cycle on that header hash
	loop {
		// if we found a cycle (not guaranteed) and the proof hash is higher that the
		// diff, we're all good
		let mut ctx = global::create_pow_context::<u32>(bh.height, sz, proof_size, MAX_SOLS)?;
		ctx.set_header_nonce(bh.pre_pow(), None, true)?;
		if let Ok(proofs) = ctx.find_cycles() {
			bh.pow.proof = proofs[0].clone();
			if bh.pow.to_difficulty(bh.height) >= diff {
				return Ok(());
			}
		}

		// otherwise increment the nonce
		let (res, _) = bh.pow.nonce.overflowing_add(1);
		bh.pow.nonce = res;

		// and if we're back where we started, update the time (changes the hash as
		// well)
		if bh.pow.nonce == start_nonce {
			bh.timestamp = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(0, 0), Utc);
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use crate::genesis;
	use crate::global;
	use crate::global::ChainTypes;

	/// We'll be generating genesis blocks differently
	#[test]
	fn genesis_pow() {
		global::set_local_chain_type(ChainTypes::UserTesting);

		let mut b = genesis::genesis_dev();
		b.header.pow.nonce = 28106;
		b.header.pow.proof.edge_bits = global::min_edge_bits();
		println!("proof {}", global::proofsize());
		pow_size(
			&mut b.header,
			Difficulty::min_dma(),
			global::proofsize(),
			global::min_edge_bits(),
		)
		.unwrap();
		println!("nonce {}", b.header.pow.nonce);
		assert_ne!(b.header.pow.nonce, 310);
		assert!(b.header.pow.to_difficulty(0) >= Difficulty::min_dma());
		assert!(verify_size(&b.header).is_ok());
	}
}
