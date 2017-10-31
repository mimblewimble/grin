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

use std::fmt;

use ser;
use core::target::Difficulty;

/// A grin is divisible to 10^9, a nanogrin
pub const GRIN_BASE: u64 = 1_000_000_000;

/// The block subsidy amount
pub const REWARD: u64 = 50 * GRIN_BASE;

/// Actual block reward for a given total fee amount
pub fn reward(fee: u64) -> u64 {
	REWARD + fee / 2
}

/// Number of blocks before a coinbase matures and can be spent
pub const COINBASE_MATURITY: u64 = 1_000;

/// Block interval, in seconds, the network will tune its next_target for. Note
/// that we may reduce this value in the future as we get more data on mining
/// with Cuckoo Cycle, networks improve and block propagation is optimized
/// (adjusting the reward accordingly).
pub const BLOCK_TIME_SEC: u64 = 60;

/// Cuckoo-cycle proof size (cycle length)
pub const PROOFSIZE: usize = 42;

/// Default Cuckoo Cycle size shift used for mining and validating.
pub const DEFAULT_SIZESHIFT: u8 = 30;

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

/// Weight of an input when counted against the max block weigth capacity
pub const BLOCK_INPUT_WEIGHT: usize = 1;

/// Weight of an output when counted against the max block weight capacity
pub const BLOCK_OUTPUT_WEIGHT: usize = 10;

/// Weight of a kernel when counted against the max block weight capacity
pub const BLOCK_KERNEL_WEIGHT: usize = 2;

/// Total maximum block weight
pub const MAX_BLOCK_WEIGHT: usize = 80_000;

/// Whether a block exceeds the maximum acceptable weight
pub fn exceeds_weight(input_len: usize, output_len: usize, kernel_len: usize) -> bool {
	input_len * BLOCK_INPUT_WEIGHT + output_len * BLOCK_OUTPUT_WEIGHT
		+ kernel_len * BLOCK_KERNEL_WEIGHT > MAX_BLOCK_WEIGHT
}

/// Fork every 250,000 blocks for first 2 years, simple number and just a
/// little less than 6 months.
pub const HARD_FORK_INTERVAL: u64 = 250_000;

/// Check whether the block version is valid at a given height, implements
/// 6 months interval scheduled hard forks for the first 2 years.
pub fn valid_header_version(height: u64, version: u16) -> bool {
	// uncomment below as we go from hard fork to hard fork
	if height <= HARD_FORK_INTERVAL && version == 1 {
		true
	/* } else if height <= 2 * HARD_FORK_INTERVAL && version == 2 {
		true */
	/* } else if height <= 3 * HARD_FORK_INTERVAL && version == 3 {
		true */
	/* } else if height <= 4 * HARD_FORK_INTERVAL && version == 4 {
		true */
	/* } else if height > 4 * HARD_FORK_INTERVAL && version > 4 {
		true */
	} else {
		false
	}
}

/// The minimum mining difficulty we'll allow
pub const MINIMUM_DIFFICULTY: u64 = 10;

/// Time window in blocks to calculate block time median
pub const MEDIAN_TIME_WINDOW: u64 = 11;

/// Number of blocks used to calculate difficulty adjustments
pub const DIFFICULTY_ADJUST_WINDOW: u64 = 23;

/// Average time span of the difficulty adjustment window
pub const BLOCK_TIME_WINDOW: u64 = DIFFICULTY_ADJUST_WINDOW * BLOCK_TIME_SEC;

/// Maximum size time window used for difficutly adjustments
pub const UPPER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW * 4 / 3;

/// Minimum size time window used for difficutly adjustments
pub const LOWER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW * 5 / 6;

/// Error when computing the next difficulty adjustment.
#[derive(Debug, Clone)]
pub struct TargetError(pub String);

impl fmt::Display for TargetError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Error computing new difficulty: {}", self.0)
	}
}

/// Computes the proof-of-work difficulty that the next block should comply
/// with. Takes an iterator over past blocks, from latest (highest height) to
/// oldest (lowest height). The iterator produces pairs of timestamp and
/// difficulty for each block.
///
/// The difficulty calculation is based on both Digishield and GravityWave
/// family of difficulty computation, coming to something very close to Zcash.
/// The refence difficulty is an average of the difficulty over a window of
/// 23 blocks. The corresponding timespan is calculated by using the
/// difference between the median timestamps at the beginning and the end
/// of the window.
pub fn next_difficulty<T>(cursor: T) -> Result<Difficulty, TargetError>
where
	T: IntoIterator<Item = Result<(u64, Difficulty), TargetError>>,
{
	// Block times at the begining and end of the adjustment window, used to
 // calculate medians later.
	let mut window_begin = vec![];
	let mut window_end = vec![];

	// Sum of difficulties in the window, used to calculate the average later.
	let mut diff_sum = Difficulty::zero();

	// Enumerating backward over blocks
	for (n, head_info) in cursor.into_iter().enumerate() {
		let m = n as u64;
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
		return Ok(Difficulty::from_num(MINIMUM_DIFFICULTY));
	}

	// Calculating time medians at the beginning and end of the window.
	window_begin.sort();
	window_end.sort();
	let begin_ts = window_begin[window_begin.len() / 2];
	let end_ts = window_end[window_end.len() / 2];

	// Average difficulty and dampened average time
	let diff_avg = diff_sum.clone() / Difficulty::from_num(DIFFICULTY_ADJUST_WINDOW);
	let ts_damp = (3 * BLOCK_TIME_WINDOW + (begin_ts - end_ts)) / 4;

	// Apply time bounds
	let adj_ts = if ts_damp < LOWER_TIME_BOUND {
		LOWER_TIME_BOUND
	} else if ts_damp > UPPER_TIME_BOUND {
		UPPER_TIME_BOUND
	} else {
		ts_damp
	};

	Ok(diff_avg * Difficulty::from_num(BLOCK_TIME_WINDOW) / Difficulty::from_num(adj_ts))
}

/// Consensus rule that collections of items are sorted lexicographically over the wire.
pub trait VerifySortOrder<T> {
	/// Verify a collection of items is sorted as required.
	fn verify_sort_order(&self) -> Result<(), ser::Error>;
}

#[cfg(test)]
use std;

#[cfg(test)]
mod test {
	use core::target::Difficulty;

	use super::*;

	// Builds an iterator for next difficulty calculation with the provided
 // constant time interval, difficulty and total length.
	fn repeat(interval: u64, diff: u64, len: u64) -> Vec<Result<(u64, Difficulty), TargetError>> {
		// watch overflow here, length shouldn't be ridiculous anyhow
		assert!(len < std::usize::MAX as u64);
		let diffs = vec![Difficulty::from_num(diff); len as usize];
		let times = (0..(len as usize)).map(|n| n * interval as usize).rev();
		let pairs = times.zip(diffs.iter());
		pairs
			.map(|(t, d)| Ok((t as u64, d.clone())))
			.collect::<Vec<_>>()
	}

	fn repeat_offs(
		from: u64,
		interval: u64,
		diff: u64,
		len: u64,
	) -> Vec<Result<(u64, Difficulty), TargetError>> {
		map_vec!(repeat(interval, diff, len), |e| match e.clone() {
			Err(e) => Err(e),
			Ok((t, d)) => Ok((t + from, d)),
		})
	}

	/// Checks different next_target adjustments and difficulty boundaries
	#[test]
	fn next_target_adjustment() {
		// not enough data
		assert_eq!(
			next_difficulty(vec![]).unwrap(),
			Difficulty::from_num(MINIMUM_DIFFICULTY)
		);

		assert_eq!(
			next_difficulty(vec![Ok((60, Difficulty::one()))]).unwrap(),
			Difficulty::from_num(MINIMUM_DIFFICULTY)
		);

		assert_eq!(
			next_difficulty(repeat(60, 10, DIFFICULTY_ADJUST_WINDOW)).unwrap(),
			Difficulty::from_num(MINIMUM_DIFFICULTY)
		);

		// just enough data, right interval, should stay constant

		let just_enough = DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW;
		assert_eq!(
			next_difficulty(repeat(60, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1000)
		);

		// checking averaging works, window length is odd so need to compensate a little
		let sec = DIFFICULTY_ADJUST_WINDOW / 2 + 1 + MEDIAN_TIME_WINDOW;
		let mut s1 = repeat(60, 500, sec);
		let mut s2 = repeat_offs((sec * 60) as u64, 60, 1545, DIFFICULTY_ADJUST_WINDOW / 2);
		s2.append(&mut s1);
		assert_eq!(next_difficulty(s2).unwrap(), Difficulty::from_num(999));

		// too slow, diff goes down
		assert_eq!(
			next_difficulty(repeat(90, 1000, just_enough)).unwrap(),
			Difficulty::from_num(889)
		);
		assert_eq!(
			next_difficulty(repeat(120, 1000, just_enough)).unwrap(),
			Difficulty::from_num(800)
		);

		// too fast, diff goes up
		assert_eq!(
			next_difficulty(repeat(55, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1021)
		);
		assert_eq!(
			next_difficulty(repeat(45, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1067)
		);

		// hitting lower time bound, should always get the same result below
		assert_eq!(
			next_difficulty(repeat(20, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1200)
		);
		assert_eq!(
			next_difficulty(repeat(10, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1200)
		);

		// hitting higher time bound, should always get the same result above
		assert_eq!(
			next_difficulty(repeat(160, 1000, just_enough)).unwrap(),
			Difficulty::from_num(750)
		);
		assert_eq!(
			next_difficulty(repeat(200, 1000, just_enough)).unwrap(),
			Difficulty::from_num(750)
		);
	}

	#[test]
	fn hard_fork_1() {
		assert!(valid_header_version(0, 1));
		assert!(valid_header_version(10, 1));
		assert!(!valid_header_version(10, 2));
		assert!(valid_header_version(250_000, 1));
		assert!(!valid_header_version(250_001, 1));
		assert!(!valid_header_version(500_000, 1));
		assert!(!valid_header_version(250_001, 2));
	}

	// #[test]
 // fn hard_fork_2() {
 // 	assert!(valid_header_version(0, 1));
 // 	assert!(valid_header_version(10, 1));
 // 	assert!(valid_header_version(10, 2));
 // 	assert!(valid_header_version(250_000, 1));
 // 	assert!(!valid_header_version(250_001, 1));
 // 	assert!(!valid_header_version(500_000, 1));
 // 	assert!(valid_header_version(250_001, 2));
 // 	assert!(valid_header_version(500_000, 2));
 // 	assert!(!valid_header_version(500_001, 2));
 // }
}
