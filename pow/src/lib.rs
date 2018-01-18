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
#[macro_use]
extern crate lazy_static;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate slog;
extern crate time;

extern crate grin_core as core;
extern crate grin_util as util;

extern crate cuckoo_miner;

mod siphash;
pub mod plugin;
pub mod cuckoo;
pub mod types;

use core::consensus;
use core::core::BlockHeader;
use core::core::hash::Hashed;
use core::core::Proof;
use core::core::target::Difficulty;
use core::global;
use core::genesis;
use cuckoo::{Cuckoo, Error};

/// Should be implemented by anything providing mining services
///

pub trait MiningWorker {
	/// This only sets parameters and does initialisation work now
	fn new(ease: u32, sizeshift: u32, proof_size: usize) -> Self
	where
		Self: Sized;

	/// Actually perform a mining attempt on the given input and
	/// return a proof if found
	fn mine(&mut self, header: &[u8]) -> Result<Proof, Error>;
}

/// Validates the proof of work of a given header, and that the proof of work
/// satisfies the requirements of the header.
pub fn verify_size(bh: &BlockHeader, cuckoo_sz: u32) -> bool {
	// make sure the pow hash shows a difficulty at least as large as the target
 // difficulty
	if bh.difficulty > bh.pow.clone().to_difficulty() {
		return false;
	}
	Cuckoo::new(&bh.hash()[..], cuckoo_sz).verify(bh.pow.clone(), consensus::EASINESS as u64)
}

/// Uses the much easier Cuckoo20 (mostly for
/// tests).
pub fn pow20<T: MiningWorker>(
	miner: &mut T,
	bh: &mut BlockHeader,
	diff: Difficulty,
) -> Result<(), Error> {
	pow_size(miner, bh, diff, 20)
}

/// Mines a genesis block, using the config specified miner if specified.
/// Otherwise, uses the internal miner
pub fn mine_genesis_block(
	miner_config: Option<types::MinerConfig>,
) -> Result<core::core::Block, Error> {
	let mut gen = genesis::genesis_dev();
	let diff = gen.header.difficulty.clone();

	let sz = global::sizeshift() as u32;
	let proof_size = global::proofsize();

	let mut miner: Box<MiningWorker> = match miner_config {
		Some(c) => if c.use_cuckoo_miner {
			let mut p = plugin::PluginMiner::new(consensus::EASINESS, sz, proof_size);
			p.init(c.clone());
			Box::new(p)
		} else {
			Box::new(cuckoo::Miner::new(consensus::EASINESS, sz, proof_size))
		},
		None => Box::new(cuckoo::Miner::new(consensus::EASINESS, sz, proof_size)),
	};
	pow_size(&mut *miner, &mut gen.header, diff, sz as u32).unwrap();
	Ok(gen)
}

/// Runs a proof of work computation over the provided block using the provided
/// Mining Worker, until the required difficulty target is reached. May take a
/// while for a low target...
pub fn pow_size<T: MiningWorker + ?Sized>(
	miner: &mut T,
	bh: &mut BlockHeader,
	diff: Difficulty,
	_: u32,
) -> Result<(), Error> {
	let start_nonce = bh.nonce;

  // set the nonce for faster solution finding in user testing
	if bh.height == 0 &&  global::is_user_testing_mode() {
    bh.nonce = global::get_genesis_nonce();
	}

  // try to find a cuckoo cycle on that header hash
  loop {
    // can be trivially optimized by avoiding re-serialization every time but this
    // is not meant as a fast miner implementation
    let pow_hash = bh.hash();

    // if we found a cycle (not guaranteed) and the proof hash is higher that the
    // diff, we're all good

    if let Ok(proof) = miner.mine(&pow_hash[..]) {
      if proof.clone().to_difficulty() >= diff {
        bh.pow = proof;
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
	use global;
	use core::core::target::Difficulty;
	use core::genesis;
	use core::consensus::MINIMUM_DIFFICULTY;
	use core::global::ChainTypes;

	#[test]
	fn genesis_pow() {
		global::set_mining_mode(ChainTypes::AutomatedTesting);
		let mut b = genesis::genesis_dev();
		b.header.nonce = 485;
		let mut internal_miner = cuckoo::Miner::new(
			consensus::EASINESS,
			global::sizeshift() as u32,
			global::proofsize(),
		);
		pow_size(
			&mut internal_miner,
			&mut b.header,
			Difficulty::from_num(MINIMUM_DIFFICULTY),
			global::sizeshift() as u32,
		).unwrap();
		assert!(b.header.nonce != 310);
		assert!(b.header.pow.clone().to_difficulty() >= Difficulty::from_num(MINIMUM_DIFFICULTY));
		assert!(verify_size(&b.header, global::sizeshift() as u32));
	}
}
