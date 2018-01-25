// Copyright 2016 The Grin Developers
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
	REWARD + fee / 2
}

/// Number of blocks before a coinbase matures and can be spent
pub const COINBASE_MATURITY: u64 = 1_000;

/// Max number of coinbase outputs in a valid block.
/// This is to prevent a miner generating an excessively large "compact block".
/// But we do techincally support blocks with multiple coinbase outputs/kernels.
pub const MAX_BLOCK_COINBASE_OUTPUTS: u64 = 1;

/// Max number of coinbase kernels in a valid block.
/// This is to prevent a miner generating an excessively large "compact block".
/// But we do techincally support blocks with multiple coinbase outputs/kernels.
pub const MAX_BLOCK_COINBASE_KERNELS: u64 = 1;

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

/// Whether a block exceeds the maximum acceptable weight
pub fn exceeds_weight(input_len: usize, output_len: usize, kernel_len: usize) -> bool {
	input_len * BLOCK_INPUT_WEIGHT + output_len * BLOCK_OUTPUT_WEIGHT
		+ kernel_len * BLOCK_KERNEL_WEIGHT > MAX_BLOCK_WEIGHT
		|| input_len > MAX_BLOCK_INPUTS
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
pub const MEDIAN_TIME_WINDOW: u64 = 12;

/// Number of blocks used to calculate difficulty adjustments
pub const DIFFICULTY_ADJUST_WINDOW: u64 = 60;

/// Average time span of the difficulty adjustment window
pub const BLOCK_TIME_WINDOW: u64 = DIFFICULTY_ADJUST_WINDOW * BLOCK_TIME_SEC;

/// Maximum size time window used for difficulty adjustments
pub const UPPER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW * 2;

/// Minimum size time window used for difficulty adjustments
pub const LOWER_TIME_BOUND: u64 = BLOCK_TIME_WINDOW / 2;

/// Dampening factor to use for difficulty adjustment
pub const DAMP_FACTOR: u64 = 2;

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
	
	// Convert iterator to vector, so we can append to it if necessary
	let needed_block_count = (MEDIAN_TIME_WINDOW + DIFFICULTY_ADJUST_WINDOW) as usize;
	let mut last_n: Vec<Result<(u64, Difficulty), TargetError>> = cursor.into_iter()
		.take(needed_block_count)
		.collect();

	if last_n.len() == 0 {
		return Err(TargetError(String::from("Difficulty data is empty.")));
	}

	// Only needed after blockchain launch... basically ensures there's
	// always enough data by simulating perfectly timed pre-genesis
	// blocks at the genesis difficulty as needed.
	let block_count_difference = needed_block_count - last_n.len();
	let earliest_ts = last_n.last().as_ref().unwrap().as_ref().unwrap().0;
	for i in 1..block_count_difference {
		last_n.push(Ok((earliest_ts - i as u64 * BLOCK_TIME_SEC, 
			Difficulty::from_num(global::initial_block_difficulty()))));
	}

	// Enumerating backward over blocks
	for (n, head_info) in last_n.into_iter().enumerate() {
		let m = n as u64;
		let (ts, diff) = head_info?;

	// Sum each element in the adjustment window. In addition, retain
	// timestamps within median windows (at ]start;start-MEDIAN_TIME_WINDOW] 
	// and ]end;end-MEDIAN_TIME_WINDOW] to later calculate medians.
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

	// Calculating time medians at the beginning and end of the window.
	window_begin.sort();
	window_end.sort();

	let begin_ts = window_begin[window_begin.len() / 2];
	let end_ts = window_end[window_end.len() / 2];

	// Average difficulty and dampened average time
	let diff_avg = diff_sum.into_num()  /
		Difficulty::from_num(DIFFICULTY_ADJUST_WINDOW).into_num();

	let ts_undamp = begin_ts - end_ts;
	let ts_damp = match diff_avg {
		n if n >= DAMP_FACTOR => ((DAMP_FACTOR-1) * BLOCK_TIME_WINDOW + ts_undamp) / DAMP_FACTOR,
		_ => ts_undamp,
	};

	// Apply time bounds
	let adj_ts = if ts_damp < LOWER_TIME_BOUND {
		LOWER_TIME_BOUND
	} else if ts_damp > UPPER_TIME_BOUND {
		UPPER_TIME_BOUND
	} else {
		ts_damp
	};

	let difficulty =
		diff_avg * Difficulty::from_num(BLOCK_TIME_WINDOW).into_num()
		/ Difficulty::from_num(adj_ts).into_num();

	Ok(max(Difficulty::from_num(difficulty), Difficulty::one()))
}

/// Consensus rule that collections of items are sorted lexicographically.
pub trait VerifySortOrder<T> {
	/// Verify a collection of items is sorted as required.
	fn verify_sort_order(&self) -> Result<(), Error>;
}

#[cfg(test)]
use std;

#[cfg(test)]
mod test {
	use core::target::Difficulty;
	use time;

	use super::*;

	// Builds an iterator for next difficulty calculation with the provided
 // constant time interval, difficulty and total length.
	fn repeat(interval: u64, diff: u64, len: u64) -> Vec<Result<(u64, Difficulty), TargetError>> {
		let cur_time =  time::get_time().sec as u64;
		// watch overflow here, length shouldn't be ridiculous anyhow
		assert!(len < std::usize::MAX as u64);
		let diffs = vec![Difficulty::from_num(diff); len as usize];
		let times = (0..(len as usize)).map(|n| n * interval as usize).rev();
		let pairs = times.zip(diffs.iter());
		pairs
			.map(|(t, d)| Ok((cur_time + t as u64, d.clone())))
			.collect::<Vec<_>>()
	}

	// Creates a new chain with a genesis at a simulated difficulty
	fn create_chain_sim(diff: u64) -> Vec<Result<(u64, Difficulty), TargetError>> {
		vec![Ok((time::get_time().sec as u64, Difficulty::from_num(diff)))]
	}

	// Adds another 'block' to the iterator, so to speak, with difficulty calculated
	// from the difficulty adjustment at interval seconds from the previous block
	fn add_block(interval: u64, chain_sim: Vec<Result<(u64, Difficulty), TargetError>>) 
		-> Vec<Result<(u64, Difficulty), TargetError>> {
		let mut return_chain = chain_sim.clone();
		let diff = next_difficulty(chain_sim).unwrap();
		// get last interval
		let last_time = return_chain.first().as_ref().unwrap().as_ref().unwrap().0;
		return_chain.insert(0, Ok((last_time+interval, diff)));
		return_chain
	}

	// Adds another n 'blocks' to the iterator, with difficulty calculated
	fn add_block_repeated(interval: u64, chain_sim: Vec<Result<(u64, Difficulty), TargetError>>, iterations: usize) 
		-> Vec<Result<(u64, Difficulty), TargetError>> {
		let mut return_chain = chain_sim.clone();
		for _ in 0..iterations {
			return_chain = add_block(interval, return_chain.clone());
		}
		return_chain
	}

	// Prints the contents of the iterator and its difficulties.. useful for tweaking
	fn print_chain_sim(chain_sim: &Vec<Result<(u64, Difficulty), TargetError>>)  {
		let mut chain_sim=chain_sim.clone();
		chain_sim.reverse();
		let mut last_time=0;
		chain_sim.iter()
			.enumerate()
			.for_each(|(i, b)| {
				let block = b.as_ref().unwrap();
				println!("Height: {}, Time: {}, Interval: {}, Next network difficulty:{}",
				i, block.0, block.0-last_time, block.1);
				last_time=block.0;
			});
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
	fn adjustment_scenarios() {
		// Use production parameters for genesis diff
		global::set_mining_mode(global::ChainTypes::Mainnet);

		// Genesis block with initial diff
		let chain_sim = create_chain_sim(global::initial_block_difficulty());
		// Scenario 1) Hash power is massively over estimated, first block takes an hour
		let chain_sim = add_block_repeated(3600, chain_sim, 10);
		let chain_sim = add_block_repeated(1800, chain_sim, 1);

		println!("*********************************************************");
		println!("Scenario 1) Grossly over-estimated genesis difficulty ");
		println!("*********************************************************");
		print_chain_sim(&chain_sim);
		println!("*********************************************************");

		let just_enough = (DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW) as usize;

	// Steady difficulty for a good while, then a sudden drop 
		let chain_sim = create_chain_sim(global::initial_block_difficulty());
		let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
		let chain_sim = add_block_repeated(600, chain_sim, 10);

		println!("");
		println!("*********************************************************");
		println!("Scenario 2) Sudden drop in hashpower");
		println!("*********************************************************");
		print_chain_sim(&chain_sim);
		println!("*********************************************************");

	// Sudden increase
		let chain_sim = create_chain_sim(global::initial_block_difficulty());
		let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
		let chain_sim = add_block_repeated(10, chain_sim, 10);

		println!("");
		println!("*********************************************************");
		println!("Scenario 3) Sudden increase in hashpower");
		println!("*********************************************************");
		print_chain_sim(&chain_sim);
		println!("*********************************************************");

	// Oscillations
		let chain_sim = create_chain_sim(global::initial_block_difficulty());
		let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
		let chain_sim = add_block_repeated(10, chain_sim, 10);
		let chain_sim = add_block_repeated(60, chain_sim, 10);
		let chain_sim = add_block_repeated(10, chain_sim, 10);
		let chain_sim = add_block_repeated(60, chain_sim, 10);

		println!("");
		println!("*********************************************************");
		println!("Scenario 4) Oscillations in hashpower");
		println!("*********************************************************");
		print_chain_sim(&chain_sim);
		println!("*********************************************************");
}

	/// Checks different next_target adjustments and difficulty boundaries
	#[test]
	fn next_target_adjustment() {
		global::set_mining_mode(global::ChainTypes::AutomatedTesting);
		let cur_time =  time::get_time().sec as u64;

		assert_eq!(
			next_difficulty(vec![Ok((cur_time, Difficulty::one()))]).unwrap(),
			Difficulty::one()
		);

		assert_eq!(
			next_difficulty(repeat(60, 1, DIFFICULTY_ADJUST_WINDOW)).unwrap(),
			Difficulty::one()
		);

		// Check we don't get stuck on difficulty 1
		assert_ne!(
			next_difficulty(repeat(1, 10, DIFFICULTY_ADJUST_WINDOW)).unwrap(),
			Difficulty::one()
		);

		// just enough data, right interval, should stay constant

		let just_enough = DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW;
		assert_eq!(
			next_difficulty(repeat(60, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1000)
		);

		// checking averaging works
		let sec = DIFFICULTY_ADJUST_WINDOW / 2 + MEDIAN_TIME_WINDOW;
		let mut s1 = repeat(60, 500, sec);
		let mut s2 = repeat_offs((sec * 60) as u64, 60, 1500, DIFFICULTY_ADJUST_WINDOW / 2);
		s2.append(&mut s1);
		assert_eq!(next_difficulty(s2).unwrap(), Difficulty::from_num(1000));

		// too slow, diff goes down
		assert_eq!(
			next_difficulty(repeat(90, 1000, just_enough)).unwrap(),
			Difficulty::from_num(800)
		);
		assert_eq!(
			next_difficulty(repeat(120, 1000, just_enough)).unwrap(),
			Difficulty::from_num(666)
		);

		// too fast, diff goes up
		assert_eq!(
			next_difficulty(repeat(55, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1043)
		);
		assert_eq!(
			next_difficulty(repeat(45, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1142)
		);

		// hitting lower time bound, should always get the same result below
		// note with the current param values, even a 1 second block interval
		// isn't enough to hit the lower bound (it comes in just above it)
		/*assert_eq!(
			next_difficulty(repeat(1, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1250)
		);
		assert_eq!(
			next_difficulty(repeat(10, 1000, just_enough)).unwrap(),
			Difficulty::from_num(1250)
		);*/

		// hitting higher time bound, should always get the same result above
		assert_eq!(
			next_difficulty(repeat(300, 1000, just_enough)).unwrap(),
			Difficulty::from_num(500)
		);
		assert_eq!(
			next_difficulty(repeat(400, 1000, just_enough)).unwrap(),
			Difficulty::from_num(500)
		);

		// We should never drop below 1
		assert_eq!(
			next_difficulty(repeat(90, 0, just_enough)).unwrap(),
			Difficulty::from_num(1)
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
