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

extern crate blake2_rfc as blake2;
extern crate rand;
extern crate serde;
extern crate time;

extern crate grin_util as util;

pub mod cuckoo;
mod siphash;

use consensus;
use core::target::Difficulty;
use core::{Block, BlockHeader};
use genesis;
use global;
use pow::cuckoo::{Cuckoo, Error};
use ser;

/// Validates the proof of work of a given header, and that the proof of work
/// satisfies the requirements of the header.
pub fn verify_size(bh: &BlockHeader, cuckoo_sz: u8) -> bool {
	Cuckoo::from_hash(bh.pre_pow_hash().as_ref(), cuckoo_sz)
		.verify(&bh.pow, consensus::EASINESS as u64)
}

/// Mines a genesis block using the internal miner
pub fn mine_genesis_block() -> Result<Block, Error> {
	let mut gen = genesis::genesis_testnet2();
	if global::is_user_testing_mode() || global::is_automated_testing_mode() {
		gen = genesis::genesis_dev();
		gen.header.timestamp = time::now();
	}

	// total_difficulty on the genesis header *is* the difficulty of that block
	let genesis_difficulty = gen.header.total_difficulty.clone();

	let sz = global::min_sizeshift();
	let proof_size = global::proofsize();

	pow_size(&mut gen.header, genesis_difficulty, proof_size, sz).unwrap();
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
	let start_nonce = bh.nonce;

	// set the nonce for faster solution finding in user testing
	if bh.height == 0 && global::is_user_testing_mode() {
		bh.nonce = global::get_genesis_nonce();
	}

	// try to find a cuckoo cycle on that header hash
	loop {
		// if we found a cycle (not guaranteed) and the proof hash is higher that the
		// diff, we're all good
		if let Ok(proof) = cuckoo::Miner::new(bh, consensus::EASINESS, proof_size, sz).mine() {
			if proof.to_difficulty() >= diff {
				bh.pow = proof.clone();
				return Ok(());
			}
		}

		// otherwise increment the nonce
		bh.nonce += 1;

		// and if we're back where we started, update the time (changes the hash as
		// well)
		if bh.nonce == start_nonce {
			bh.timestamp = time::at_utc(time::Timespec { sec: 0, nsec: 0 });
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::target::Difficulty;
	use genesis;
	use global;

	/// We'll be generating genesis blocks differently
	#[ignore]
	#[test]
	fn genesis_pow() {
		let mut b = genesis::genesis_dev();
		b.header.nonce = 485;
		pow_size(
			&mut b.header,
			Difficulty::one(),
			global::proofsize(),
			global::min_sizeshift(),
		).unwrap();
		assert!(b.header.nonce != 310);
		assert!(b.header.pow.to_difficulty() >= Difficulty::one());
		assert!(verify_size(&b.header, global::min_sizeshift()));
	}
}
