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

//! All the rules required for a cryptocurrency to have reach consensus across
//! the whole network are complex and hard to completely isolate. Some can be
//! simple parameters (like block reward), others complex algorithms (like
//! Merkle sum trees or reorg rules). However, as long as they're simple
//! enough, consensus-relevant constants and short functions should be kept
//! here.

use std::cmp;

use bigint::{BigInt, Sign, BigUint};

use core::target::Difficulty;

/// The block subsidy amount
pub const REWARD: u64 = 1_000_000_000;

/// Block interval, in seconds, the network will tune its next_target for. Note
/// that we may reduce this value in the future as we get more data on mining
/// with Cuckoo Cycle, networks improve and block propagation is optimized
/// (adjusting the reward accordingly).
pub const BLOCK_TIME_SEC: u8 = 60;

/// Cuckoo-cycle proof size (cycle length)
pub const PROOFSIZE: usize = 42;

/// Origin Cuckoo Cycle size shift used by the genesis block.
pub const DEFAULT_SIZESHIFT: u8 = 25;

/// Maximum Cuckoo Cycle size shift we'll ever use. We adopt a schedule that
/// progressively increases the size as the target becomes lower.
///   Start => 25
///   MAX_TARGET >> 12 => 26
///   MAX_TARGET >> 20 => 27
///   MAX_TARGET >> 28 => 28
///   MAX_TARGET >> 36 => 29
pub const MAX_SIZESHIFT: u8 = 29;

/// Default Cuckoo Cycle easiness, high enough to have good likeliness to find
/// a solution.
pub const EASINESS: u32 = 50;

/// Difficulty adjustment somewhat inspired by Ethereum's. Tuned to add or
/// remove 1/1024th of the target for each 10 seconds of deviation from the 30
/// seconds block time. Increases Cuckoo size shift by one when next_target
/// reaches soft max.
pub fn next_target(ts: i64,
                   prev_ts: i64,
                   prev_diff: Difficulty,
                   prev_cuckoo_sz: u8)
                   -> (Difficulty, u8) {
	let one = BigInt::new(Sign::Plus, vec![1]);
	let two = BigInt::new(Sign::Plus, vec![2]);
	let ten = BigInt::new(Sign::Plus, vec![10]);

	// increase the cuckoo size when the target gets lower than the soft min as
	// long as we're not at the max size already; target gets 2x to compensate for
	// increased next_target
	let soft_min = one.clone() <<
	               (((prev_cuckoo_sz - cmp::min(DEFAULT_SIZESHIFT, prev_cuckoo_sz)) *
	                 8 + 16) as usize);
	let prev_diff = BigInt::from_biguint(Sign::Plus, prev_diff.num);
	let (pdiff, clen) = if prev_diff > soft_min && prev_cuckoo_sz < MAX_SIZESHIFT {
		(prev_diff / two, prev_cuckoo_sz + 1)
	} else {
		(prev_diff, prev_cuckoo_sz)
	};

	// signed deviation from desired value divided by ten and bounded in [-6, 6]
	let delta = cmp::max(cmp::min((ts - prev_ts - (BLOCK_TIME_SEC as i64)), 60), -60);
	let delta_bigi = BigInt::new(if delta >= 0 { Sign::Plus } else { Sign::Minus },
	                             vec![delta.abs() as u32]);
	let new_diff = pdiff.clone() - ((pdiff >> 10) + one.clone()) * delta_bigi / ten;

	// cannot be lower than one
	if new_diff < one {
		(Difficulty::one(), clen)
	} else {
		(Difficulty { num: new_diff.to_biguint().unwrap() }, clen)
	}
}

/// Default number of blocks in the past when cross-block cut-through will start
/// happening. Needs to be long enough to not overlap with a long reorg.
/// Rational
/// behind the value is the longest bitcoin fork was about 30 blocks, so 5h. We
/// add an order of magnitude to be safe and round to 48h of blocks to make it
/// easier to reason about.
pub const CUT_THROUGH_HORIZON: u32 = 48 * 3600 / (BLOCK_TIME_SEC as u32);

/// The maximum size we're willing to accept for any message. Enforced by the
/// peer-to-peer networking layer only for DoS protection.
pub const MAX_MSG_LEN: u64 = 20_000_000;

#[cfg(test)]
mod test {
	use core::target::Difficulty;

	use super::*;

	#[test]
	/// Checks different next_target adjustments and difficulty boundaries
	fn next_target_adjustment() {
		// can't do lower than min
		assert_eq!(next_target(60, 0, Difficulty::one(), 26),
		           (Difficulty::one(), 26));
		assert_eq!(next_target(90, 30, Difficulty::one(), 26),
		           (Difficulty::one(), 26));
		assert_eq!(next_target(60, 0, Difficulty::one(), 26),
		           (Difficulty::one(), 26));

		// lower next_target if gap too short
		assert_eq!(next_target(30, 0, Difficulty::one(), 26).0,
		           Difficulty::from_num(4));
		assert_eq!(next_target(50, 0, Difficulty::one(), 26).0,
		           Difficulty::from_num(2));
		assert_eq!(next_target(40, 0, Difficulty::from_num(1024 * 8), 26).0,
		           Difficulty::from_num(1024 * 8 + 18));

		// lower difficulty if gap too wide
		assert_eq!(next_target(70, 0, Difficulty::from_num(10), 26).0,
		           Difficulty::from_num(9));
		assert_eq!(next_target(90, 0, Difficulty::from_num(1024 * 8), 26).0,
		           Difficulty::from_num(1024 * 8 - 9 * 3));

		// identical, no adjustment
		assert_eq!(next_target(60, 0, Difficulty::from_num(1024 * 8), 26).0,
		           Difficulty::from_num(1024 * 8));

		// increase cuckoo size if next_target goes above soft max, target is doubled,
		// up to 29
		assert_eq!(next_target(60, 0, Difficulty::from_num(1 << 16), 25),
		           (Difficulty::from_num(1 << 16), 25));
		assert_eq!(next_target(60, 0, Difficulty::from_num((1 << 16) + 1), 25),
		           (Difficulty::from_num(1 << 15), 26));
		assert_eq!(next_target(60, 0, Difficulty::from_num((1 << 24) + 1), 26),
		           (Difficulty::from_num(1 << 23), 27));
	}
}
