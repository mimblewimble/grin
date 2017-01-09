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

mod siphash;
pub mod cuckoo;

use time;

use consensus::EASINESS;
use core::{Block, Proof};
use core::hash::{Hash, Hashed};
use core::target::Difficulty;
use pow::cuckoo::{Cuckoo, Miner, Error};

use ser;
use ser::{Writeable, Writer};

/// Validates the proof of work of a given header.
pub fn verify(b: &Block) -> bool {
	verify_size(b, b.header.cuckoo_len as u32)
}

pub fn verify_size(b: &Block, cuckoo_sz: u32) -> bool {
	// make sure the pow hash shows a difficulty at least as large as the target
	// difficulty
	if b.header.difficulty > b.header.pow.to_difficulty() {
		return false;
	}
	Cuckoo::new(b.hash().to_slice(), cuckoo_sz).verify(b.header.pow, EASINESS as u64)
}

/// Runs a naive single-threaded proof of work computation over the provided
/// block, until the required difficulty target is reached. May take a
/// while for a low target...
pub fn pow(b: &mut Block, diff: Difficulty) -> Result<(), Error> {
	let cuckoo_len = b.header.cuckoo_len as u32;
	pow_size(b, diff, cuckoo_len)
}

/// Same as default pow function but uses the much easier Cuckoo20 (mostly for
/// tests).
pub fn pow20(b: &mut Block, diff: Difficulty) -> Result<(), Error> {
	pow_size(b, diff, 20)
}

pub fn pow_size(b: &mut Block, diff: Difficulty, sizeshift: u32) -> Result<(), Error> {
	let start_nonce = b.header.nonce;

	// try to find a cuckoo cycle on that header hash
	loop {
		// can be trivially optimized by avoiding re-serialization every time but this
		// is not meant as a fast miner implementation
		let pow_hash = b.hash();

		// if we found a cycle (not guaranteed) and the proof hash is higher that the
		// diff, we're all good
		if let Ok(proof) = Miner::new(pow_hash.to_slice(), EASINESS, sizeshift).mine() {
			if proof.to_difficulty() >= diff {
				b.header.pow = proof;
				return Ok(());
			}
		}

		// otherwise increment the nonce
		b.header.nonce += 1;

		// and if we're back where we started, update the time (changes the hash as
		// well)
		if b.header.nonce == start_nonce {
			b.header.timestamp = time::at_utc(time::Timespec { sec: 0, nsec: 0 });
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use core::Proof;
	use core::target::Difficulty;
	use genesis;

	#[test]
	fn genesis_pow() {
		let mut b = genesis::genesis();
		b.header.nonce = 310;
		pow20(&mut b, Difficulty::one()).unwrap();
		assert!(b.header.nonce != 310);
		assert!(b.header.pow.to_difficulty() >= Difficulty::one());
		assert!(verify_size(&b, 20));
	}
}
