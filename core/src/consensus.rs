// Copyright 2018 The Grin Developers
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
use std::cmp::max;

use core::target::Difficulty;
use global;

/// A grin is divisible to 10^9, following the SI prefixes
pub const GRIN_BASE: u64 = 1_000_000_000;
/// Milligrin, a thousand of a grin
pub const MILLI_GRIN: u64 = GRIN_BASE / 1_000;
/// Microgrin, a thousand of a milligrin
pub const MICRO_GRIN: u64 = MILLI_GRIN / 1_000;
/// Nanogrin, smallest unit, takes a billion to make a grin
pub const NANO_GRIN: u64 = 1;

/// The block subsidy amount, one grin per second on average
pub const REWARD: u64 = 60 * GRIN_BASE;

/// Actual block reward for a given total fee amount
pub fn reward(fee: u64) -> u64 {
	REWARD + fee
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

/// Maximum inputs for a block (issue#261)
/// Hundreds of inputs + 1 output might be slow to validate (issue#258)
pub const MAX_BLOCK_INPUTS: usize = 300_000; // soft fork down when too_high

/// Maixmum inputs for a transaction
pub const MAX_TX_INPUTS: u64 = 500;

/// Maixmum outputs for a transaction
pub const MAX_TX_OUTPUTS: u64 = 500; // wallet uses 500 as max

/// Maixmum kernels for a transaction
pub const MAX_TX_KERNELS: u64 = 2048;

/// Whether a block exceeds the maximum acceptable weight
pub fn exceeds_weight(input_len: usize, output_len: usize, kernel_len: usize) -> bool {
	input_len * BLOCK_INPUT_WEIGHT + output_len * BLOCK_OUTPUT_WEIGHT
		+ kernel_len * BLOCK_KERNEL_WEIGHT > MAX_BLOCK_WEIGHT || input_len > MAX_BLOCK_INPUTS
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

/// Time window in blocks to calculate block time median
pub const MEDIAN_TIME_WINDOW: u64 = 11;

/// Index at half the desired median
pub const MEDIAN_TIME_INDEX: u64 = MEDIAN_TIME_WINDOW / 2;

/// Number of blocks used to calculate difficulty adjustments
pub const DIFFICULTY_ADJUST_WINDOW: u64 = 60;

/// Average time span of the difficulty adjustment window
pub const BLOCK_TIME_WINDOW: u64 = DIFFICULTY_ADJUST_WINDOW * BLOCK_TIME_SEC;

/// Maximum size time window used for difficulty adjustments
pub const UPPER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW * 2;

/// Minimum size time window used for difficulty adjustments
pub const LOWER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW / 2;

/// Dampening factor to use for difficulty adjustment
pub const DAMP_FACTOR: u64 = 3;

/// The initial difficulty at launch. This should be over-estimated
/// and difficulty should come down at launch rather than up
/// Currently grossly over-estimated at 10% of current
/// ethereum GPUs (assuming 1GPU can solve a block at diff 1
/// in one block interval)
pub const INITIAL_DIFFICULTY: u64 = 1_000_000;

/// Consensus errors
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
	/// Inputs/outputs/kernels must be sorted lexicographically.
	SortError,
}

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
/// DIFFICULTY_ADJUST_WINDOW blocks. The corresponding timespan is calculated
/// by using the difference between the median timestamps at the beginning
/// and the end of the window.
pub fn next_difficulty<T>(cursor: T) -> Result<Difficulty, TargetError>
where
	T: IntoIterator<Item = Result<(u64, Difficulty), TargetError>>,
{
	// Create vector of difficulty data running from earliest
	// to latest, and pad with simulated pre-genesis data to allow earlier
	// adjustment if there isn't enough window data
	// length will be DIFFICULTY_ADJUST_WINDOW+MEDIAN_TIME_WINDOW
	let diff_data = global::difficulty_data_to_vector(cursor);
	// Get the difficulty sum for averaging later
	// Which in this case is the sum of the last
	// DIFFICULTY_ADJUST_WINDOW elements
	let diff_sum = diff_data
		.iter()
		.skip(MEDIAN_TIME_WINDOW as usize)
		.take(DIFFICULTY_ADJUST_WINDOW as usize)
		.fold(Difficulty::zero(), |sum, d| sum + d.clone().unwrap().1);

	// Obtain the median window for the earlier time period
	// which is just the first MEDIAN_TIME_WINDOW elements
	let mut window_earliest: Vec<u64> = diff_data
		.iter()
		.take(MEDIAN_TIME_WINDOW as usize)
		.map(|n| n.clone().unwrap().0)
		.collect();

	// Obtain the median window for the latest time period
	// i.e. the last MEDIAN_TIME_WINDOW elements
	let mut window_latest: Vec<u64> = diff_data
		.iter()
		.skip(DIFFICULTY_ADJUST_WINDOW as usize)
		.map(|n| n.clone().unwrap().0)
		.collect();

	// And obtain our median values
	window_earliest.sort();
	window_latest.sort();
	let latest_ts = window_latest[MEDIAN_TIME_INDEX as usize];
	let earliest_ts = window_earliest[MEDIAN_TIME_INDEX as usize];

	// Calculate the average difficulty
	let diff_avg = diff_sum.into_num() / Difficulty::from_num(DIFFICULTY_ADJUST_WINDOW).into_num();

	// Actual undampened time delta
	let ts_delta = latest_ts - earliest_ts;

	// Apply dampening
	let ts_damp = match diff_avg {
		n if n >= DAMP_FACTOR => ((DAMP_FACTOR - 1) * BLOCK_TIME_WINDOW + ts_delta) / DAMP_FACTOR,
		_ => ts_delta,
	};

	// Apply time bounds
	let adj_ts = if ts_damp < LOWER_TIME_BOUND {
		LOWER_TIME_BOUND
	} else if ts_damp > UPPER_TIME_BOUND {
		UPPER_TIME_BOUND
	} else {
		ts_damp
	};

	let difficulty = diff_avg * Difficulty::from_num(BLOCK_TIME_WINDOW).into_num()
		/ Difficulty::from_num(adj_ts).into_num();

	Ok(max(Difficulty::from_num(difficulty), Difficulty::one()))
}

/// Consensus rule that collections of items are sorted lexicographically.
pub trait VerifySortOrder<T> {
	/// Verify a collection of items is sorted as required.
	fn verify_sort_order(&self) -> Result<(), Error>;
}
