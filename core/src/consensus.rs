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

use core::target::Target;

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
pub fn next_target(ts: i64, prev_ts: i64, prev_target: Target, prev_cuckoo_sz: u8) -> (Target, u8) {
	// increase the cuckoo size when the target gets lower than the soft min as
	// long as we're not at the max size already; target gets 2x to compensate for
	// increased next_target
	let soft_min = SOFT_MIN_TARGET >> (((prev_cuckoo_sz - cmp::min(DEFAULT_SIZESHIFT, prev_cuckoo_sz)) * 8) as usize);
	let (ptarget, clen) = if prev_target < soft_min && prev_cuckoo_sz < MAX_SIZESHIFT {
		(prev_target << 1, prev_cuckoo_sz + 1)
	} else {
		(prev_target, prev_cuckoo_sz)
	};

	// target is increased/decreased by multiples of 1/1024th of itself
	let incdec = ptarget >> 10;
	// signed deviation from desired value divided by ten and bounded in [-3, 3]
	let delta = cmp::max(cmp::min((ts - prev_ts - (BLOCK_TIME_SEC as i64)) / 10, 3),
	                     -3);
	// increase or decrease the target based on the sign of delta by a shift of
	// |delta|-1; keep as-is for delta of zero
	let new_target = match delta {
		1...3 => ptarget + (incdec << ((delta - 1) as usize)),
		-3...-1 => ptarget - (incdec << ((-delta - 1) as usize)),
		_ => ptarget,
	};

	// cannot exceed the maximum target
	if new_target > MAX_TARGET {
		(MAX_TARGET, clen)
	} else {
		(new_target.truncate(), clen)
	}
}

/// Max target hash, lowest next_target
pub const MAX_TARGET: Target = Target([0xf, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0,
                                       0, 0, 0, 0, 0, 0, 0, 0, 0,
                                       0, 0, 0, 0, 0, 0, 0, 0, 0,
                                       0, 0, 0, 0]);

/// Target limit under which we start increasing the size shift on Cuckoo cycle.
pub const SOFT_MIN_TARGET: Target = Target([0, 0, 0xf, 0xff, 0xff, 0xff, 0, 0, 0, 0,
                                       0, 0, 0, 0, 0, 0, 0, 0, 0,
                                       0, 0, 0, 0, 0, 0, 0, 0, 0,
                                       0, 0, 0, 0]);

/// Default number of blocks in the past when cross-block cut-through will start
/// happening. Needs to be long enough to not overlap with a long reorg.
/// Rational
/// behind the value is the longest bitcoin fork was about 30 blocks, so 5h. We
/// add an order of magnitude to be safe and round to 48h of blocks to make it
/// easier to reason about.
pub const CUT_THROUGH_HORIZON: u32 = 48 * 3600 / (BLOCK_TIME_SEC as u32);

/// The maximum number of inputs or outputs a transaction may have
/// and be deserializable. Only for DoS protection.
pub const MAX_IN_OUT_LEN: u64 = 50000;

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	/// Checks different next_target adjustments and difficulty boundaries
	fn next_target_adjustment() {
		// can't do lower than min
		assert_eq!(next_target(60, 0, MAX_TARGET, 26), (MAX_TARGET, 26));
		assert_eq!(next_target(90, 30, MAX_TARGET, 26), (MAX_TARGET, 26));
		assert_eq!(next_target(60, 0, MAX_TARGET, 26), (MAX_TARGET, 26));

		// lower next_target if gap too short, even negative
		assert_eq!(next_target(50, 0, MAX_TARGET, 26).0,
		           (MAX_TARGET - (MAX_TARGET >> 10)).truncate());
		assert_eq!(next_target(40, 0, MAX_TARGET, 26).0,
		           (MAX_TARGET - ((MAX_TARGET >> 10) << 1)).truncate());
		assert_eq!(next_target(0, 20, MAX_TARGET, 26).0,
		           (MAX_TARGET - ((MAX_TARGET >> 10) << 2)).truncate());

		// raise next_target if gap too wide
		let lower_target = MAX_TARGET >> 8;
		assert_eq!(next_target(70, 0, lower_target, 26).0,
		           (lower_target + (lower_target >> 10)).truncate());
		assert_eq!(next_target(80, 0, lower_target, 26).0,
		           (lower_target + ((lower_target >> 10) << 1)).truncate());
		assert_eq!(next_target(200, 0, lower_target, 26).0,
		           (lower_target + ((lower_target >> 10) << 2)).truncate());

		// close enough, no adjustment
		assert_eq!(next_target(65, 0, lower_target, 26).0, lower_target);
		assert_eq!(next_target(55, 0, lower_target, 26).0, lower_target);

		// increase cuckoo size if next_target goes above soft max, target is doubled,
		// up to 29
		assert_eq!(next_target(60, 0, SOFT_MIN_TARGET >> 1, 25),
		           ((SOFT_MIN_TARGET >> 1) << 1, 26));
		assert_eq!(next_target(60, 0, SOFT_MIN_TARGET >> 9, 26),
		           ((SOFT_MIN_TARGET >> 9) << 1, 27));
		assert_eq!(next_target(60, 0, SOFT_MIN_TARGET >> 17, 27),
		           ((SOFT_MIN_TARGET >> 17) << 1, 28));
		assert_eq!(next_target(60, 0, SOFT_MIN_TARGET >> 25, 28),
		           ((SOFT_MIN_TARGET >> 25) << 1, 29));
		assert_eq!(next_target(60, 0, SOFT_MIN_TARGET >> 33, 29),
		           (SOFT_MIN_TARGET >> 33, 29));

		// should only increase on the according previous size
		assert_eq!(next_target(60, 0, SOFT_MIN_TARGET >> 1, 26),
		           (SOFT_MIN_TARGET >> 1, 26));
	}
}
