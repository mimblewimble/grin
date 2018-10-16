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

use std::cmp::{max, min};
use std::fmt;

use global;
use pow::Difficulty;

/// A grin is divisible to 10^9, following the SI prefixes
pub const GRIN_BASE: u64 = 1_000_000_000;
/// Milligrin, a thousand of a grin
pub const MILLI_GRIN: u64 = GRIN_BASE / 1_000;
/// Microgrin, a thousand of a milligrin
pub const MICRO_GRIN: u64 = MILLI_GRIN / 1_000;
/// Nanogrin, smallest unit, takes a billion to make a grin
pub const NANO_GRIN: u64 = 1;

/// Block interval, in seconds, the network will tune its next_target for. Note
/// that we may reduce this value in the future as we get more data on mining
/// with Cuckoo Cycle, networks improve and block propagation is optimized
/// (adjusting the reward accordingly).
pub const BLOCK_TIME_SEC: u64 = 60;

/// The block subsidy amount, one grin per second on average
pub const REWARD: u64 = BLOCK_TIME_SEC * GRIN_BASE;

/// Actual block reward for a given total fee amount
pub fn reward(fee: u64) -> u64 {
	REWARD + fee
}

/// Nominal height for standard time intervals
pub const HOUR_HEIGHT: u64 = 3600 / BLOCK_TIME_SEC;
pub const  DAY_HEIGHT: u64 = 24 * HOUR_HEIGHT;
pub const WEEK_HEIGHT: u64 =  7 *  DAY_HEIGHT;
pub const YEAR_HEIGHT: u64 = 52 * WEEK_HEIGHT;

/// Number of blocks before a coinbase matures and can be spent
pub const COINBASE_MATURITY: u64 = DAY_HEIGHT;

/// Ratio the secondary proof of work should take over the primary, as a
/// function of block height (time). Starts at 90% losing a percent
/// approximately every week. Represented as an integer between 0 and 100.
pub fn secondary_pow_ratio(height: u64) -> u64 {
	90u64.saturating_sub(height / WEEK_HEIGHT)
}

/// Cuckoo-cycle proof size (cycle length)
pub const PROOFSIZE: usize = 42;

/// Default Cuckoo Cycle edge_bits, used for mining and validating.
pub const DEFAULT_MIN_EDGE_BITS: u8 = 30;

/// Secondary proof-of-work edge_bits, meant to be ASIC resistant.
pub const SECOND_POW_EDGE_BITS: u8 = 29;

/// Original reference edge_bits to compute difficulty factors for higher
/// Cuckoo graph sizes, changing this would hard fork
pub const BASE_EDGE_BITS: u8 = 24;

/// Maximum scaling factor for secondary pow, enforced in diff retargetting
/// increasing scaling factor increases frequency of secondary blocks
/// ONLY IN TESTNET4 LIMITED TO ABOUT 8 TIMES THE NATURAL SCALE
pub const MAX_SECONDARY_SCALING: u64 = 8 << 11;

/// Default number of blocks in the past when cross-block cut-through will start
/// happening. Needs to be long enough to not overlap with a long reorg.
/// Rational
/// behind the value is the longest bitcoin fork was about 30 blocks, so 5h. We
/// add an order of magnitude to be safe and round to 7x24h of blocks to make it
/// easier to reason about.
pub const CUT_THROUGH_HORIZON: u32 = WEEK_HEIGHT as u32;

/// Weight of an input when counted against the max block weight capacity
pub const BLOCK_INPUT_WEIGHT: usize = 1;

/// Weight of an output when counted against the max block weight capacity
pub const BLOCK_OUTPUT_WEIGHT: usize = 10;

/// Weight of a kernel when counted against the max block weight capacity
pub const BLOCK_KERNEL_WEIGHT: usize = 2;

/// Total maximum block weight. At current sizes, this means a maximum
/// theoretical size of:
/// * `(674 + 33 + 1) * 4_000 = 2_832_000` for a block with only outputs
/// * `(1 + 8 + 8 + 33 + 64) * 20_000 = 2_280_000` for a block with only kernels
/// * `(1 + 33) * 40_000 = 1_360_000` for a block with only inputs
///
/// Given that a block needs to have at least one kernel for the coinbase,
/// and one kernel for the transaction, practical maximum size is 2_831_440,
/// (ignoring the edge case of a miner producing a block with all coinbase
/// outputs and a single kernel).
///
/// A more "standard" block, filled with transactions of 2 inputs, 2 outputs
/// and one kernel, should be around 2.66 MB
pub const MAX_BLOCK_WEIGHT: usize = 40_000;

/// Fork every 6 months.
pub const HARD_FORK_INTERVAL: u64 = YEAR_HEIGHT / 2;

/// Check whether the block version is valid at a given height, implements
/// 6 months interval scheduled hard forks for the first 2 years.
pub fn valid_header_version(height: u64, version: u16) -> bool {
	// uncomment below as we go from hard fork to hard fork
	if height < HARD_FORK_INTERVAL {
		version == 1
	/* } else if height < 2 * HARD_FORK_INTERVAL {
		version == 2
	} else if height < 3 * HARD_FORK_INTERVAL {
		version == 3
	} else if height < 4 * HARD_FORK_INTERVAL {
		version == 4 
	} else if height >= 5 * HARD_FORK_INTERVAL {
		version > 4 */
	} else {
		false
	}
}

/// Time window in blocks to calculate block time median
pub const MEDIAN_TIME_WINDOW: u64 = 11;

/// Index at half the desired median
pub const MEDIAN_TIME_INDEX: u64 = MEDIAN_TIME_WINDOW / 2;

/// Number of blocks used to calculate difficulty adjustments
pub const DIFFICULTY_ADJUST_WINDOW: u64 = HOUR_HEIGHT;

/// Average time span of the difficulty adjustment window
pub const BLOCK_TIME_WINDOW: u64 = DIFFICULTY_ADJUST_WINDOW * BLOCK_TIME_SEC;

/// Maximum size time window used for difficulty adjustments
pub const UPPER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW * 2;

/// Minimum size time window used for difficulty adjustments
pub const LOWER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW / 2;

/// Dampening factor to use for difficulty adjustment
pub const DAMP_FACTOR: u64 = 3;

/// Compute difficulty scaling factor as number of siphash bits defining the graph
/// Must be made dependent on height to phase out smaller size over the years
/// This can wait until end of 2019 at latest
pub fn scale(edge_bits: u8) -> u64 {
	(2 << (edge_bits - global::base_edge_bits()) as u64) * (edge_bits as u64)
}

/// The initial difficulty at launch. This should be over-estimated
/// and difficulty should come down at launch rather than up
/// Currently grossly over-estimated at 10% of current
/// ethereum GPUs (assuming 1GPU can solve a block at diff 1 in one block interval)
/// Pick MUCH more modest value for TESTNET4; CHANGE FOR MAINNET
pub const INITIAL_DIFFICULTY: u64 = 1_000 * (2<<(29-24)) * 29; // scale(SECOND_POW_EDGE_BITS);
/// pub const INITIAL_DIFFICULTY: u64 = 1_000_000 * Difficulty::scale(SECOND_POW_EDGE_BITS);

/// Consensus errors
#[derive(Clone, Debug, Eq, PartialEq, Fail)]
pub enum Error {
	/// Inputs/outputs/kernels must be sorted lexicographically.
	SortError,
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Sort Error")
	}
}

/// Minimal header information required for the Difficulty calculation to
/// take place
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeaderInfo {
	/// Timestamp of the header, 1 when not used (returned info)
	pub timestamp: u64,
	/// Network difficulty or next difficulty to use
	pub difficulty: Difficulty,
	/// Network secondary PoW factor or factor to use
	pub secondary_scaling: u32,
	/// Whether the header is a secondary proof of work
	pub is_secondary: bool,
}

impl HeaderInfo {
	/// Default constructor
	pub fn new(
		timestamp: u64,
		difficulty: Difficulty,
		secondary_scaling: u32,
		is_secondary: bool,
	) -> HeaderInfo {
		HeaderInfo {
			timestamp,
			difficulty,
			secondary_scaling,
			is_secondary,
		}
	}

	/// Constructor from a timestamp and difficulty, setting a default secondary
	/// PoW factor
	pub fn from_ts_diff(timestamp: u64, difficulty: Difficulty) -> HeaderInfo {
		HeaderInfo {
			timestamp,
			difficulty,
			secondary_scaling: 1,
			is_secondary: false,
		}
	}

	/// Constructor from a difficulty and secondary factor, setting a default
	/// timestamp
	pub fn from_diff_scaling(difficulty: Difficulty, secondary_scaling: u32) -> HeaderInfo {
		HeaderInfo {
			timestamp: 1,
			difficulty,
			secondary_scaling,
			is_secondary: false,
		}
	}
}

/// Computes the proof-of-work difficulty that the next block should comply
/// with. Takes an iterator over past block headers information, from latest
/// (highest height) to oldest (lowest height).
///
/// The difficulty calculation is based on both Digishield and GravityWave
/// family of difficulty computation, coming to something very close to Zcash.
/// The reference difficulty is an average of the difficulty over a window of
/// DIFFICULTY_ADJUST_WINDOW blocks. The corresponding timespan is calculated
/// by using the difference between the median timestamps at the beginning
/// and the end of the window.
///
/// The secondary proof-of-work factor is calculated along the same lines, as
/// an adjustment on the deviation against the ideal value.
pub fn next_difficulty<T>(height: u64, cursor: T) -> HeaderInfo
where
	T: IntoIterator<Item = HeaderInfo>,
{
	// Create vector of difficulty data running from earliest
	// to latest, and pad with simulated pre-genesis data to allow earlier
	// adjustment if there isn't enough window data
	// length will be DIFFICULTY_ADJUST_WINDOW+MEDIAN_TIME_WINDOW
	let diff_data = global::difficulty_data_to_vector(cursor);

	// First, get the ratio of secondary PoW vs primary
	let sec_pow_scaling = secondary_pow_scaling(height, &diff_data);

	// Obtain the median window for the earlier time period
	// the first MEDIAN_TIME_WINDOW elements
	let earliest_ts = time_window_median(&diff_data, 0, MEDIAN_TIME_WINDOW as usize);

	// Obtain the median window for the latest time period
	// i.e. the last MEDIAN_TIME_WINDOW elements
	let latest_ts = time_window_median(
		&diff_data,
		DIFFICULTY_ADJUST_WINDOW as usize,
		MEDIAN_TIME_WINDOW as usize,
	);

	// median time delta
	let ts_delta = latest_ts - earliest_ts;

	// Get the difficulty sum of the last DIFFICULTY_ADJUST_WINDOW elements
	let diff_sum = diff_data
		.iter()
		.skip(MEDIAN_TIME_WINDOW as usize)
		.fold(0, |sum, d| sum + d.difficulty.to_num());

	// Apply dampening except when difficulty is near 1
	let ts_damp = if diff_sum < DAMP_FACTOR * DIFFICULTY_ADJUST_WINDOW {
		ts_delta
	} else {
		(1 * ts_delta + (DAMP_FACTOR - 1) * BLOCK_TIME_WINDOW) / DAMP_FACTOR
	};

	// Apply time bounds
	let adj_ts = if ts_damp < LOWER_TIME_BOUND {
		LOWER_TIME_BOUND
	} else if ts_damp > UPPER_TIME_BOUND {
		UPPER_TIME_BOUND
	} else {
		ts_damp
	};

	let difficulty = max(diff_sum * BLOCK_TIME_SEC / adj_ts, 1);

	HeaderInfo::from_diff_scaling(Difficulty::from_num(difficulty), sec_pow_scaling)
}

/// Factor by which the secondary proof of work difficulty will be adjusted
pub fn secondary_pow_scaling(height: u64, diff_data: &Vec<HeaderInfo>) -> u32 {
	// median of past scaling factors, scaling is 1 if none found
	let mut scalings = diff_data
		.iter()
		.map(|n| n.secondary_scaling)
		.collect::<Vec<_>>();
	if scalings.len() == 0 {
		return 1;
	}
	scalings.sort();
	let scaling_median = scalings[scalings.len() / 2] as u64;
	let secondary_count = max(diff_data.iter().filter(|n| n.is_secondary).count(), 1) as u64;

	// what's the ideal ratio at the current height
	let ratio = secondary_pow_ratio(height);

	// adjust the past median based on ideal ratio vs actual ratio
	let scaling = scaling_median * diff_data.len() as u64 * ratio / 100 / secondary_count as u64;

	// various bounds
	let bounded_scaling = if scaling < scaling_median / 2 || scaling == 0 {
		max(scaling_median / 2, 1)
	} else if scaling > MAX_SECONDARY_SCALING || scaling > scaling_median * 2 {
		min(MAX_SECONDARY_SCALING, scaling_median * 2)
	} else {
		scaling
	};
	bounded_scaling as u32
}

/// Median timestamp within the time window starting at `from` with the
/// provided `length`.
fn time_window_median(diff_data: &Vec<HeaderInfo>, from: usize, length: usize) -> u64 {
	let mut window_latest: Vec<u64> = diff_data
		.iter()
		.skip(from)
		.take(length)
		.map(|n| n.timestamp)
		.collect();
	// pick median
	window_latest.sort();
	window_latest[MEDIAN_TIME_INDEX as usize]
}

/// Consensus rule that collections of items are sorted lexicographically.
pub trait VerifySortOrder<T> {
	/// Verify a collection of items is sorted as required.
	fn verify_sort_order(&self) -> Result<(), Error>;
}
