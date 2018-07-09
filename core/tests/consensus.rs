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

//! core consensus.rs tests (separated to de-clutter consensus.rs)
#[macro_use]
extern crate grin_core as core;
extern crate time;

use core::consensus::{next_difficulty, valid_header_version, TargetError,
                      DIFFICULTY_ADJUST_WINDOW, MEDIAN_TIME_WINDOW,
                      UPPER_TIME_BOUND, BLOCK_TIME_WINDOW, DAMP_FACTOR, MEDIAN_TIME_INDEX};
use core::core::target::Difficulty;
use std::fmt::{self, Display};
use core::global;

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
	/// earlist ts
	pub earliest_ts: u64,
	/// ts delta
	pub ts_delta: u64,
	
}

impl Display for DiffBlock {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let output = format!(
			"Block Number: {} Difficulty: {}, Time: {}, Duration: {}",
			self.block_number, self.difficulty, self.time, self.duration
		);
		Display::fmt(&output, f)
	}
}

// Builds an iterator for next difficulty calculation with the provided
// constant time interval, difficulty and total length.
fn repeat(
	interval: u64,
	diff: u64,
	len: u64,
	cur_time: Option<u64>,
) -> Vec<Result<(u64, Difficulty), TargetError>> {
	let cur_time = match cur_time {
		Some(t) => t,
		None => time::get_time().sec as u64,
	};
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
fn create_chain_sim(diff: u64)
	-> Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)> {
	println!(
		"adding create: {}, {}",
		time::get_time().sec,
		Difficulty::from_num(diff)
	);
	let return_vec = vec![
		Ok((time::get_time().sec as u64, Difficulty::from_num(diff))),
	];
	let diff_stats = get_diff_stats(&return_vec);
	vec![
		(Ok((time::get_time().sec as u64, Difficulty::from_num(diff))), diff_stats),
	]
}

fn get_diff_stats(chain_sim: &Vec<Result<(u64, Difficulty), TargetError>>) -> DiffStats {
	// Fill out some difficulty stats for convenience
	let diff_iter = chain_sim.clone();
	let last_blocks: Vec<Result<(u64, Difficulty), TargetError>> =
		global::difficulty_data_to_vector(diff_iter.clone());

	let mut last_time = last_blocks[0].clone().unwrap().0;
	let tip_height = chain_sim.len();
	let earliest_block_height = tip_height as i64 - last_blocks.len() as i64;

	// Obtain the median window for the earlier time period
	// the first MEDIAN_TIME_WINDOW elements
	let mut window_earliest: Vec<u64> = last_blocks.clone()
		.iter()
		.take(MEDIAN_TIME_WINDOW as usize)
		.map(|n| n.clone().unwrap().0)
		.collect();
	// pick median
	window_earliest.sort();
	let earliest_ts = window_earliest[MEDIAN_TIME_INDEX as usize];

	// Obtain the median window for the latest time period
	// i.e. the last  MEDIAN_TIME_WINDOW elements
	let mut window_latest: Vec<u64> = last_blocks.clone()
		.iter()
		.skip(DIFFICULTY_ADJUST_WINDOW as usize)
		.map(|n| n.clone().unwrap().0)
		.collect();
	// pick median
	window_latest.sort();
	let latest_ts = window_latest[MEDIAN_TIME_INDEX as usize];

	let mut i = 1;
	
	let sum_blocks: Vec<Result<(u64, Difficulty), TargetError>> =
		global::difficulty_data_to_vector(diff_iter)
			.into_iter()
			.skip(MEDIAN_TIME_WINDOW as usize)
			.take(DIFFICULTY_ADJUST_WINDOW as usize)
			.collect();

	let sum_entries: Vec<DiffBlock> = sum_blocks
		.iter()
		//.skip(1)
		.map(|n| {
			let (time, diff) = n.clone().unwrap();
			let dur = time - last_time;
			let height = earliest_block_height + i + 1;
			i += 1;
			last_time = time;
			DiffBlock {
				block_number: height,
				difficulty: diff.to_num(),
				time: time,
				duration: dur,
			}
		})
		.collect();

	let block_time_sum = sum_entries.iter().fold(0, |sum, t| sum + t.duration);
	let block_diff_sum = sum_entries.iter().fold(0, |sum, d| sum + d.difficulty);

	i = 1;
	last_time = last_blocks[0].clone().unwrap().0;

	let diff_entries: Vec<DiffBlock> = last_blocks
		.iter()
		.skip(1)
		.map(|n| {
			let (time, diff) = n.clone().unwrap();
			let dur = time - last_time;
			let height = earliest_block_height + i;
			i += 1;
			last_time = time;
			DiffBlock {
				block_number: height,
				difficulty: diff.to_num(),
				time: time,
				duration: dur,
			}
		})
		.collect();

	DiffStats {
		height: tip_height as u64,
		last_blocks: diff_entries,
		average_block_time: block_time_sum / (DIFFICULTY_ADJUST_WINDOW),
		average_difficulty: block_diff_sum / (DIFFICULTY_ADJUST_WINDOW),
		window_size: DIFFICULTY_ADJUST_WINDOW,
		block_time_sum: block_time_sum,
		block_diff_sum: block_diff_sum,
		latest_ts: latest_ts,
		earliest_ts: earliest_ts,
		ts_delta: latest_ts-earliest_ts,
	}
}

// Adds another 'block' to the iterator, so to speak, with difficulty calculated
// from the difficulty adjustment at interval seconds from the previous block
fn add_block(
	interval: u64,
	chain_sim: Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)>,
) -> Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)> {
	let mut ret_chain_sim = chain_sim.clone();
	let mut return_chain: Vec<(Result<(u64, Difficulty), TargetError>)> = chain_sim.clone().iter()
		.map(|e| e.0.clone())
		.collect();
	// get last interval
	let diff = next_difficulty(return_chain.clone()).unwrap();
	let last_elem = chain_sim.first().as_ref().unwrap().0.as_ref().unwrap();
	let time = last_elem.0 + interval;
	return_chain.insert(0, Ok((time, diff)));
	let diff_stats = get_diff_stats(&return_chain);
	ret_chain_sim.insert(0, (Ok((time, diff)), diff_stats));
	ret_chain_sim
}

// Adds many defined blocks
fn add_blocks(
	intervals: Vec<u64>,
	chain_sim: Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)>,
) -> Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)> {
	let mut return_chain = chain_sim.clone();
	for i in intervals {
		return_chain = add_block(i, return_chain.clone());
	}
	return_chain
}

// Adds another n 'blocks' to the iterator, with difficulty calculated
fn add_block_repeated(
	interval: u64,
	chain_sim: Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)>,
	iterations: usize,
) -> Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)> {
	let mut return_chain = chain_sim.clone();
	for _ in 0..iterations {
		return_chain = add_block(interval, return_chain.clone());
	}
	return_chain
}

// Prints the contents of the iterator and its difficulties.. useful for
// tweaking
fn print_chain_sim(
	chain_sim: Vec<((Result<(u64, Difficulty), TargetError>), DiffStats)>
){
	let mut chain_sim = chain_sim.clone();
	chain_sim.reverse();
	let mut last_time = 0;
	let mut first = true;
	println!("Constants");
	println!("DIFFICULTY_ADJUST_WINDOW: {}", DIFFICULTY_ADJUST_WINDOW);
	println!("BLOCK_TIME_WINDOW: {}", BLOCK_TIME_WINDOW);
	println!("MEDIAN_TIME_WINDOW: {}", MEDIAN_TIME_WINDOW);
	println!("UPPER_TIME_BOUND: {}", UPPER_TIME_BOUND);
	println!("DAMP_FACTOR: {}", DAMP_FACTOR);
	chain_sim.iter().enumerate().for_each(|(i, b)| {
		let block = b.0.as_ref().unwrap();
		let stats = b.1.clone();
		if first {
			last_time = block.0;
			first = false;
		}
		println!(
			"Height: {}, Time: {}, Interval: {}, Network difficulty:{}, Average Block Time: {}, Average Difficulty {}, Block Time Sum: {}, Block Diff Sum: {}, Latest Timestamp: {}, Earliest Timestamp: {}, Timestamp Delta: {}",
			i,
			block.0,
			block.0 - last_time,
			block.1,
			stats.average_block_time,
			stats.average_difficulty,
			stats.block_time_sum,
			stats.block_diff_sum,
			stats.latest_ts,
			stats.earliest_ts,
			stats.ts_delta,
		);
		let mut sb = stats.last_blocks.clone();
		sb.reverse();
		for i in sb {
			println!("   {}", i);
		}
		last_time = block.0;
	});
}

fn repeat_offs(
	from: u64,
	interval: u64,
	diff: u64,
	len: u64,
) -> Vec<Result<(u64, Difficulty), TargetError>> {
	map_vec!(
		repeat(interval, diff, len, Some(from)),
		|e| match e.clone() {
			Err(e) => Err(e),
			Ok((t, d)) => Ok((t, d)),
		}
	)
}

/// Checks different next_target adjustments and difficulty boundaries
#[test]
fn adjustment_scenarios() {
	// Use production parameters for genesis diff
	global::set_mining_mode(global::ChainTypes::Mainnet);

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
	let just_enough = (DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW) as usize;

	// Steady difficulty for a good while, then a sudden drop
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
	let chain_sim = add_block_repeated(600, chain_sim, 60);

	println!("");
	println!("*********************************************************");
	println!("Scenario 3) Sudden drop in hashpower");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");

	// Sudden increase
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_block_repeated(60, chain_sim, just_enough as usize);
	let chain_sim = add_block_repeated(10, chain_sim, 10);

	println!("");
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

	println!("");
	println!("*********************************************************");
	println!("Scenario 5) Oscillations in hashpower");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");

	// Actual testnet 2 timings
	let testnet2_intervals = [
		2880, 16701, 1882, 3466, 614, 605, 1551, 538, 931, 23, 690, 1397, 2112, 2058, 605, 721,
		2148, 1605, 134, 1234, 1569, 482, 1775, 2732, 540, 958, 883, 3475, 518, 1346, 1926, 780,
		865, 269, 1079, 141, 105, 781, 289, 256, 709, 68, 165, 1813, 3899, 1458, 955, 2336, 239,
		674, 1059, 157, 214, 15, 157, 558, 1945, 1677, 1825, 1307, 1973, 660, 77, 3134, 410, 347,
		537, 649, 325, 370, 2271, 106, 19, 329,
	];

	global::set_mining_mode(global::ChainTypes::Testnet2);
	let chain_sim = create_chain_sim(global::initial_block_difficulty());
	let chain_sim = add_blocks(testnet2_intervals.to_vec(), chain_sim);

	println!("");
	println!("*********************************************************");
	println!("Scenario 6) Testnet2");
	println!("*********************************************************");
	print_chain_sim(chain_sim);
	println!("*********************************************************");
}

/// Checks different next_target adjustments and difficulty boundaries
#[test]
fn next_target_adjustment() {
	global::set_mining_mode(global::ChainTypes::AutomatedTesting);
	let cur_time = time::get_time().sec as u64;

	assert_eq!(
		next_difficulty(vec![Ok((cur_time, Difficulty::one()))]).unwrap(),
		Difficulty::one()
	);

	assert_eq!(
		next_difficulty(repeat(60, 1, DIFFICULTY_ADJUST_WINDOW, None)).unwrap(),
		Difficulty::one()
	);

	// Check we don't get stuck on difficulty 1
	assert_ne!(
		next_difficulty(repeat(1, 10, DIFFICULTY_ADJUST_WINDOW, None)).unwrap(),
		Difficulty::one()
	);

	// just enough data, right interval, should stay constant
	let just_enough = DIFFICULTY_ADJUST_WINDOW + MEDIAN_TIME_WINDOW;
	assert_eq!(
		next_difficulty(repeat(60, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(1000)
	);

	// checking averaging works
	let sec = DIFFICULTY_ADJUST_WINDOW / 2 + MEDIAN_TIME_WINDOW;
	let mut s1 = repeat(60, 500, sec, Some(cur_time));
	let mut s2 = repeat_offs(
		cur_time + (sec * 60) as u64,
		60,
		1500,
		DIFFICULTY_ADJUST_WINDOW / 2,
	);
	s2.append(&mut s1);
	assert_eq!(next_difficulty(s2).unwrap(), Difficulty::from_num(1000));

	// too slow, diff goes down
	assert_eq!(
		next_difficulty(repeat(90, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(857)
	);
	assert_eq!(
		next_difficulty(repeat(120, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(750)
	);

	// too fast, diff goes up
	assert_eq!(
		next_difficulty(repeat(55, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(1028)
	);
	assert_eq!(
		next_difficulty(repeat(45, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(1090)
	);

	// hitting lower time bound, should always get the same result below
	assert_eq!(
		next_difficulty(repeat(0, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(1500)
	);
	assert_eq!(
		next_difficulty(repeat(0, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(1500)
	);

	// hitting higher time bound, should always get the same result above
	assert_eq!(
		next_difficulty(repeat(300, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(500)
	);
	assert_eq!(
		next_difficulty(repeat(400, 1000, just_enough, None)).unwrap(),
		Difficulty::from_num(500)
	);

	// We should never drop below 1
	assert_eq!(
		next_difficulty(repeat(90, 0, just_enough, None)).unwrap(),
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
