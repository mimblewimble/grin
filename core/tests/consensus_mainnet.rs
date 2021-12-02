// Copyright 2021 The Grin Developers
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

//! core consensus mainnet tests (separated to de-clutter consensus_mainnet)

//! Setting global::mining_mode() changes global shared state so automated tests should only use one
//! mining mode/chain type per test file to avoid non-deterministic behaviour.

use grin_core as core;

use self::core::consensus::*;
use self::core::core::block::HeaderVersion;
use self::core::global;
use self::core::pow::Difficulty;
use chrono::prelude::Utc;
use std::fmt::{self, Display};

/// Last n blocks for difficulty calculation purposes
/// (copied from stats in server crate)
#[derive(Clone, Debug)]
pub struct DiffBlock {
	/// Block number (can be negative for a new chain)
	pub block_number: i64,
	/// Block network difficulty
	pub difficulty: u64,
	/// Time block was found (epoch seconds)
	pub time: u64,
	/// Duration since previous block (epoch seconds)
	pub duration: u64,
}

/// Stats on the last WINDOW blocks and the difficulty calculation
/// (Copied from stats in server crate)
#[derive(Clone)]
pub struct DiffStats {
	/// latest height
	pub height: u64,
	/// Last WINDOW block data
	pub last_blocks: Vec<DiffBlock>,
	/// Average block time for last WINDOW blocks
	pub average_block_time: u64,
	/// Average WINDOW difficulty
	pub average_difficulty: u64,
	/// WINDOW size
	pub window_size: u64,
	/// Block time sum
	pub block_time_sum: u64,
	/// Block diff sum
	pub block_diff_sum: u64,
	/// latest ts
	pub latest_ts: u64,
	/// earliest ts
	pub earliest_ts: u64,
	/// ts delta
	pub ts_delta: u64,
}

impl Display for DiffBlock {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let output = format!(
			"Block Number: {} Difficulty: {}, Time: {}, Duration: {}",
			self.block_number, self.difficulty, self.time, self.duration
		);
		Display::fmt(&output, f)
	}
}

// Creates a new chain with a genesis at a simulated difficulty
fn create_chain_sim(diff: u64) -> Vec<(HeaderDifficultyInfo, DiffStats)> {
	println!(
		"adding create: {}, {}",
		Utc::now().timestamp(),
		Difficulty::from_num(diff)
	);
	let return_vec = vec![HeaderDifficultyInfo::from_ts_diff(
		Utc::now().timestamp() as u64,
		Difficulty::from_num(diff),
	)];
	let diff_stats = get_diff_stats(&return_vec);
	vec![(
		HeaderDifficultyInfo::from_ts_diff(
			Utc::now().timestamp() as u64,
			Difficulty::from_num(diff),
		),
		diff_stats,
	)]
}

fn get_diff_stats(chain_sim: &[HeaderDifficultyInfo]) -> DiffStats {
	// Fill out some difficulty stats for convenience
	let diff_iter = chain_sim.to_vec();
	let last_blocks: Vec<HeaderDifficultyInfo> =
		global::difficulty_data_to_vector(diff_iter.iter().cloned());

	let mut last_time = last_blocks[0].timestamp;
	let tip_height = chain_sim.len();
	let earliest_block_height = tip_height as i64 - last_blocks.len() as i64;

	let earliest_ts = last_blocks[0].timestamp;
	let latest_ts = last_blocks[last_blocks.len() - 1].timestamp;

	let mut i = 1;

	let sum_blocks: Vec<HeaderDifficultyInfo> =
		global::difficulty_data_to_vector(diff_iter.iter().cloned())
			.into_iter()
			.take(DMA_WINDOW as usize)
			.collect();

	let sum_entries: Vec<DiffBlock> = sum_blocks
		.iter()
		//.skip(1)
		.map(|n| {
			let dur = n.timestamp - last_time;
			let height = earliest_block_height + i + 1;
			i += 1;
			last_time = n.timestamp;
			DiffBlock {
				block_number: height,
				difficulty: n.difficulty.to_num(),
				time: n.timestamp,
				duration: dur,
			}
		})
		.collect();

	let block_time_sum = sum_entries.iter().fold(0, |sum, t| sum + t.duration);
	let block_diff_sum = sum_entries.iter().fold(0, |sum, d| sum + d.difficulty);

	i = 1;
	last_time = last_blocks[0].clone().timestamp;

	let diff_entries: Vec<DiffBlock> = last_blocks
		.iter()
		.skip(1)
		.map(|n| {
			let dur = n.timestamp - last_time;
			let height = earliest_block_height + i;
			i += 1;
			last_time = n.timestamp;
			DiffBlock {
				block_number: height,
				difficulty: n.difficulty.to_num(),
				time: n.timestamp,
				duration: dur,
			}
		})
		.collect();

	DiffStats {
		height: tip_height as u64,
		last_blocks: diff_entries,
		average_block_time: block_time_sum / DMA_WINDOW,
		average_difficulty: block_diff_sum / DMA_WINDOW,
		window_size: DMA_WINDOW,
		block_time_sum: block_time_sum,
		block_diff_sum: block_diff_sum,
		latest_ts: latest_ts,
		earliest_ts: earliest_ts,
		ts_delta: latest_ts - earliest_ts,
	}
}

// Adds another 'block' to the iterator, so to speak, with difficulty calculated
// from the difficulty adjustment at interval seconds from the previous block
fn add_block(
	interval: u64,
	chain_sim: Vec<(HeaderDifficultyInfo, DiffStats)>,
) -> Vec<(HeaderDifficultyInfo, DiffStats)> {
	let mut ret_chain_sim = chain_sim.clone();
	let mut return_chain: Vec<HeaderDifficultyInfo> =
		chain_sim.clone().iter().map(|e| e.0.clone()).collect();
	// get last interval
	let diff = next_difficulty(1, return_chain.clone());
	let last_elem = chain_sim.first().unwrap().clone().0;
	let time = last_elem.timestamp + interval;
	return_chain.insert(0, HeaderDifficultyInfo::from_ts_diff(time, diff.difficulty));
	let diff_stats = get_diff_stats(&return_chain);
	ret_chain_sim.insert(
		0,
		(
			HeaderDifficultyInfo::from_ts_diff(time, diff.difficulty),
			diff_stats,
		),
	);
	ret_chain_sim
}

// Adds another n 'blocks' to the iterator, with difficulty calculated
fn add_block_repeated(
	interval: u64,
	chain_sim: Vec<(HeaderDifficultyInfo, DiffStats)>,
	iterations: usize,
) -> Vec<(HeaderDifficultyInfo, DiffStats)> {
	let mut return_chain = chain_sim;
	for _ in 0..iterations {
		return_chain = add_block(interval, return_chain.clone());
	}
	return_chain
}

// Prints the contents of the iterator and its difficulties.. useful for
// tweaking
fn print_chain_sim(chain_sim: Vec<(HeaderDifficultyInfo, DiffStats)>) {
	let mut chain_sim = chain_sim;
	chain_sim.reverse();
	let mut last_time = 0;
	let mut first = true;
	println!("Constants");
	println!("DIFFICULTY_ADJUST_WINDOW: {}", DMA_WINDOW);
	println!("BLOCK_TIME_WINDOW: {}", BLOCK_TIME_WINDOW);
	println!("CLAMP_FACTOR: {}", CLAMP_FACTOR);
	println!("DAMP_FACTOR: {}", DMA_DAMP_FACTOR);
	chain_sim.iter().enumerate().for_each(|(i, b)| {
		let block = b.0.clone();
		let stats = b.1.clone();
		if first {
			last_time = block.timestamp;
			first = false;
		}
		println!(
			"Height: {}, Time: {}, Interval: {}, Network difficulty:{}, Average Block Time: {}, Average Difficulty {}, Block Time Sum: {}, Block Diff Sum: {}, Latest Timestamp: {}, Earliest Timestamp: {}, Timestamp Delta: {}",
			i,
			block.timestamp,
			block.timestamp - last_time,
			block.difficulty,
			stats.average_block_time,
			stats.average_difficulty,
			stats.block_time_sum,
			stats.block_diff_sum,
			stats.latest_ts,
			stats.earliest_ts,
			stats.ts_delta,
		);
		let mut sb = stats.last_blocks;
		sb.reverse();
		for i in sb {
			println!("   {}", i);
		}
		last_time = block.timestamp;
	});
}

/// Checks different next_target adjustments and difficulty boundaries
#[test]
fn adjustment_scenarios() {
	global::set_local_chain_type(global::ChainTypes::Mainnet);

	// Genesis block with initial diff
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	// Scenario 1) Hash power is massively over estimated, first block takes an hour
	let chain_sim = add_block_repeated(3600, chain_sim, 2);
	let chain_sim = add_block_repeated(1800, chain_sim, 2);
	let chain_sim = add_block_repeated(900, chain_sim, 10);
	let chain_sim = add_block_repeated(450, chain_sim, 30);
	let chain_sim = add_block_repeated(400, chain_sim, 30);
	let chain_sim = add_block_repeated(300, chain_sim, 30);

	println!("*********************************************************");
	println!("Scenario 1) Grossly over-estimated genesis difficulty ");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");

	// Under-estimated difficulty
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_block_repeated(1, chain_sim, 5);
	let chain_sim = add_block_repeated(20, chain_sim, 5);
	let chain_sim = add_block_repeated(30, chain_sim, 20);

	println!("*********************************************************");
	println!("Scenario 2) Grossly under-estimated genesis difficulty ");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");
	let just_enough = DMA_WINDOW as usize;

	// Steady difficulty for a good while, then a sudden drop
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
	let chain_sim = add_block_repeated(600, chain_sim, 60);

	println!();
	println!("*********************************************************");
	println!("Scenario 3) Sudden drop in hashpower");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");

	// Sudden increase
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
	let chain_sim = add_block_repeated(10, chain_sim, 10);

	println!();
	println!("*********************************************************");
	println!("Scenario 4) Sudden increase in hashpower");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");

	// Oscillations
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
	let chain_sim = add_block_repeated(10, chain_sim, 10);
	let chain_sim = add_block_repeated(60, chain_sim, 20);
	let chain_sim = add_block_repeated(10, chain_sim, 10);

	println!();
	println!("*********************************************************");
	println!("Scenario 5) Oscillations in hashpower");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");
}

#[test]
fn test_secondary_pow_ratio() {
	global::set_local_chain_type(global::ChainTypes::Mainnet);

	assert_eq!(secondary_pow_ratio(1), 90);
	assert_eq!(secondary_pow_ratio(89), 90);
	assert_eq!(secondary_pow_ratio(90), 90);
	assert_eq!(secondary_pow_ratio(91), 90);
	assert_eq!(secondary_pow_ratio(179), 90);
	assert_eq!(secondary_pow_ratio(180), 90);
	assert_eq!(secondary_pow_ratio(181), 90);

	let one_week = 60 * 24 * 7;
	assert_eq!(secondary_pow_ratio(one_week - 1), 90);
	assert_eq!(secondary_pow_ratio(one_week), 90);
	assert_eq!(secondary_pow_ratio(one_week + 1), 90);

	let two_weeks = one_week * 2;
	assert_eq!(secondary_pow_ratio(two_weeks - 1), 89);
	assert_eq!(secondary_pow_ratio(two_weeks), 89);
	assert_eq!(secondary_pow_ratio(two_weeks + 1), 89);

	let t4_fork_height = 64_000;
	assert_eq!(secondary_pow_ratio(t4_fork_height - 1), 85);
	assert_eq!(secondary_pow_ratio(t4_fork_height), 85);
	assert_eq!(secondary_pow_ratio(t4_fork_height + 1), 85);

	let one_year = one_week * 52;
	assert_eq!(secondary_pow_ratio(one_year), 45);

	let ninety_one_weeks = one_week * 91;
	assert_eq!(secondary_pow_ratio(ninety_one_weeks - 1), 12);
	assert_eq!(secondary_pow_ratio(ninety_one_weeks), 12);
	assert_eq!(secondary_pow_ratio(ninety_one_weeks + 1), 12);

	let two_year = one_year * 2;
	assert_eq!(secondary_pow_ratio(two_year - 1), 1);
	assert_eq!(secondary_pow_ratio(two_year), 0);
	assert_eq!(secondary_pow_ratio(two_year + 1), 0);
}

#[test]
fn test_secondary_pow_scale() {
	global::set_local_chain_type(global::ChainTypes::Mainnet);

	let window = DMA_WINDOW;
	let mut hi = HeaderDifficultyInfo::from_diff_scaling(Difficulty::from_num(10), 100);

	// all primary, factor should increase so it becomes easier to find a high
	// difficulty block
	hi.is_secondary = false;
	assert_eq!(
		secondary_pow_scaling(1, &(0..window).map(|_| hi.clone()).collect::<Vec<_>>()),
		108
	);
	// all secondary on 90%, factor should go down a bit
	hi.is_secondary = true;
	assert_eq!(
		secondary_pow_scaling(1, &(0..window).map(|_| hi.clone()).collect::<Vec<_>>()),
		99
	);
	// all secondary on 1%, factor should go down to bound (divide by 2)
	assert_eq!(
		secondary_pow_scaling(
			2 * YEAR_HEIGHT * 83 / 90,
			&(0..window).map(|_| hi.clone()).collect::<Vec<_>>()
		),
		50
	);
	// same as above, testing lowest bound
	let mut low_hi =
		HeaderDifficultyInfo::from_diff_scaling(Difficulty::from_num(10), MIN_AR_SCALE as u32);
	low_hi.is_secondary = true;
	assert_eq!(
		secondary_pow_scaling(
			2 * YEAR_HEIGHT,
			&(0..window).map(|_| low_hi.clone()).collect::<Vec<_>>()
		),
		MIN_AR_SCALE as u32
	);
	// the right ratio of 95% secondary
	let mut primary_hi = HeaderDifficultyInfo::from_diff_scaling(Difficulty::from_num(10), 50);
	primary_hi.is_secondary = false;
	assert_eq!(
		secondary_pow_scaling(
			1,
			&(0..(window / 10))
				.map(|_| primary_hi.clone())
				.chain((0..(window * 9 / 10)).map(|_| hi.clone()))
				.collect::<Vec<_>>()
		),
		95, // avg ar_scale of 10% * 50 + 90% * 100
	);
	// 95% secondary, should come down based on 97.5 average
	assert_eq!(
		secondary_pow_scaling(
			1,
			&(0..(window / 20))
				.map(|_| primary_hi.clone())
				.chain((0..(window * 95 / 100)).map(|_| hi.clone()))
				.collect::<Vec<_>>()
		),
		97
	);
	// 40% secondary, should come up based on 70 average
	assert_eq!(
		secondary_pow_scaling(
			1,
			&(0..(window * 6 / 10))
				.map(|_| primary_hi.clone())
				.chain((0..(window * 4 / 10)).map(|_| hi.clone()))
				.collect::<Vec<_>>()
		),
		73
	);
}

#[test]
fn hard_forks() {
	global::set_local_chain_type(global::ChainTypes::Mainnet);

	assert_eq!(header_version(0), HeaderVersion(1));
	assert_eq!(header_version(10), HeaderVersion(1));

	assert_eq!(header_version(HARD_FORK_INTERVAL - 1), HeaderVersion(1));
	assert_eq!(header_version(HARD_FORK_INTERVAL), HeaderVersion(2));
	assert_eq!(header_version(HARD_FORK_INTERVAL + 1), HeaderVersion(2));

	assert_eq!(header_version(HARD_FORK_INTERVAL * 2 - 1), HeaderVersion(2));
	assert_eq!(header_version(HARD_FORK_INTERVAL * 2), HeaderVersion(3));
	assert_eq!(header_version(HARD_FORK_INTERVAL * 2 + 1), HeaderVersion(3));

	assert_eq!(header_version(HARD_FORK_INTERVAL * 3 - 1), HeaderVersion(3));
	assert_eq!(header_version(HARD_FORK_INTERVAL * 3), HeaderVersion(4));
	assert_eq!(header_version(HARD_FORK_INTERVAL * 3 + 1), HeaderVersion(4));

	assert_eq!(header_version(HARD_FORK_INTERVAL * 4 - 1), HeaderVersion(4));
	assert_eq!(header_version(HARD_FORK_INTERVAL * 4), HeaderVersion(5));
	assert_eq!(header_version(HARD_FORK_INTERVAL * 4 + 1), HeaderVersion(5));
}
