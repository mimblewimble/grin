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

use bigint::{BigInt, Sign};

use core::target::Difficulty;

/// The block subsidy amount
pub const REWARD: u64 = 1_000_000_000;

/// Block interval, in seconds, the network will tune its next_target for. Note
/// that we may reduce this value in the future as we get more data on mining
/// with Cuckoo Cycle, networks improve and block propagation is optimized
/// (adjusting the reward accordingly).
pub const BLOCK_TIME_SEC: i64 = 60;

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

pub const MEDIAN_TIME_WINDOW: u32 = 11;

pub const DIFFICULTY_ADJUST_WINDOW: u32 = 23;

pub const BLOCK_TIME_WINDOW: i64 = (DIFFICULTY_ADJUST_WINDOW as i64) * BLOCK_TIME_SEC;

pub const UPPER_TIME_BOUND: i64 = BLOCK_TIME_WINDOW * 4 / 3;

pub const LOWER_TIME_BOUND: i64 = BLOCK_TIME_WINDOW * 5 / 6;

#[derive(Debug, Clone)]
pub struct TargetError {
	err: String,
}
pub fn next_target2<T>(cursor: T) -> Result<Difficulty, TargetError>
	where T: IntoIterator<Item = Result<(i64, Difficulty), TargetError>>
{

	// Block times at the begining and end of the adjustment window, used to
	// calculate medians later.
	let mut window_begin = vec![];
	let mut window_end = vec![];

	// Sum of difficulties in the window, used to calculate the average later.
	let mut diff_sum = Difficulty::zero();

	// Enumerating backward over blocks
	for (n, head_info) in cursor.into_iter().enumerate() {
		let m = n as u32;
		let (ts, diff) = head_info?;

		// Sum each element in the adjustment window. In addition, retain
		// timestamps within median windows (at ]start;start-11] and ]end;end-11]
		// to later calculate medians.
		if m < DIFFICULTY_ADJUST_WINDOW {
			diff_sum = diff_sum + diff;

			if m < MEDIAN_TIME_WINDOW {
				window_begin.push(ts);
			}
		} else if m < DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW {
			window_end.push(ts);
		} else {
			break;
		}
	}

	// Check we have enough blocks
	if window_end.len() < (MEDIAN_TIME_WINDOW as usize) {
		return Ok(Difficulty::one());
	}

	// Calculating time medians at the beginning and end of the window.
	window_begin.sort();
	window_end.sort();
	let begin_ts = window_begin[window_begin.len() / 2];
	let end_ts = window_end[window_end.len() / 2];

	// Average difficulty and dampened average time
	let diff_avg = diff_sum / Difficulty::from_num(DIFFICULTY_ADJUST_WINDOW);
	let ts_damp = (3 * BLOCK_TIME_WINDOW + (begin_ts - end_ts)) / 4;

	// Apply time bounds
	let adj_ts = if ts_damp < LOWER_TIME_BOUND {
		LOWER_TIME_BOUND
	} else if ts_damp > UPPER_TIME_BOUND {
		UPPER_TIME_BOUND
	} else {
		ts_damp
	};

	// Final ratio calculation
	Ok(diff_avg * Difficulty::from_num(BLOCK_TIME_WINDOW as u32) /
	   Difficulty::from_num(adj_ts as u32))
}

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
	let prev_diff = BigInt::from_biguint(Sign::Plus, prev_diff.into_biguint());
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
		(Difficulty::from_biguint(new_diff.to_biguint().unwrap()), clen)
	}
}

#[cfg(test)]
mod test {
	use core::target::Difficulty;

	use super::*;

	// Builds an iterator for next difficulty calculation with the provided
	// constant time interval, difficulty and total length.
	fn repeat(interval: i64, diff: u32, len: u32) -> Vec<Result<(i64, Difficulty), TargetError>> {
		let diffs = vec![Difficulty::from_num(diff); len as usize];
		let times = (0..(len as usize)).map(|n| (n as i64) * interval).rev();
		let pairs = times.zip(diffs.iter());
		pairs.map(|(t, d)| Ok((t, d.clone()))).collect::<Vec<_>>()
	}

	fn repeat_offs(from: i64,
	               interval: i64,
	               diff: u32,
	               len: u32)
	               -> Vec<Result<(i64, Difficulty), TargetError>> {
		map_vec!(repeat(interval, diff, len), |e| {
			match e.clone() {
				Err(e) => Err(e),
				Ok((t, d)) => Ok((t + from, d)),
			}
		})
	}

	/// Checks different next_target adjustments and difficulty boundaries
	#[test]
	fn next_target_adjustment() {
		// not enough data
		assert_eq!(next_target2(vec![]).unwrap(), Difficulty::one());
		assert_eq!(next_target2(vec![Ok((60, Difficulty::one()))]).unwrap(),
		           Difficulty::one());
		assert_eq!(next_target2(repeat(60, 10, DIFFICULTY_ADJUST_WINDOW)).unwrap(),
		           Difficulty::one());

		// just enough data, right interval, should stay constant
		let just_enough = DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW;
		assert_eq!(next_target2(repeat(60, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(1000));

		// checking averaging works, window length is odd so need to compensate a little
		let sec = DIFFICULTY_ADJUST_WINDOW / 2 + 1 + MEDIAN_TIME_WINDOW;
		let mut s1 = repeat(60, 500, sec);
		let mut s2 = repeat_offs((sec * 60) as i64, 60, 1545, DIFFICULTY_ADJUST_WINDOW / 2);
		s2.append(&mut s1);
		assert_eq!(next_target2(s2).unwrap(), Difficulty::from_num(999));

		// too slow, diff goes down
		assert_eq!(next_target2(repeat(90, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(889));
		assert_eq!(next_target2(repeat(120, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(800));

		// too fast, diff goes up
		assert_eq!(next_target2(repeat(55, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(1021));
		assert_eq!(next_target2(repeat(45, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(1067));

		// hitting lower time bound, should always get the same result below
		assert_eq!(next_target2(repeat(20, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(1200));
		assert_eq!(next_target2(repeat(10, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(1200));

		// hitting higher time bound, should always get the same result above
		assert_eq!(next_target2(repeat(160, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(750));
		assert_eq!(next_target2(repeat(200, 1000, just_enough)).unwrap(),
		           Difficulty::from_num(750));
	}

}
