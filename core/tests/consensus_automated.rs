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

use chrono::Utc;
use grin_core::consensus::{
	next_dma_difficulty, next_wtema_difficulty, HeaderDifficultyInfo, AR_SCALE_DAMP_FACTOR,
	BLOCK_TIME_SEC, DMA_WINDOW, MIN_AR_SCALE, YEAR_HEIGHT,
};
use grin_core::global;
use grin_core::pow::Difficulty;

/// Checks different next_dma_difficulty adjustments and difficulty boundaries
#[test]
fn next_dma_difficulty_adjustment() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	let cur_time = Utc::now().timestamp() as u64;
	let diff_min = Difficulty::min_dma();

	// Check we don't get stuck on difficulty <= Difficulty::min_dma (at 4x faster blocks at least)
	let mut hi = HeaderDifficultyInfo::from_diff_scaling(diff_min, AR_SCALE_DAMP_FACTOR as u32);
	hi.is_secondary = false;
	let hinext = next_dma_difficulty(1, repeat(BLOCK_TIME_SEC / 4, hi.clone(), DMA_WINDOW, None));

	assert_ne!(hinext.difficulty, diff_min);

	// Check we don't get stuck on scale MIN_AR_SCALE, when primary frequency is too high
	assert_ne!(hinext.secondary_scaling, MIN_AR_SCALE as u32);

	// just enough data, right interval, should stay constant
	let just_enough = DMA_WINDOW + 1;
	hi.difficulty = Difficulty::from_num(10000);
	assert_eq!(
		next_dma_difficulty(1, repeat(BLOCK_TIME_SEC, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(10000)
	);

	// check pre difficulty_data_to_vector effect on retargetting
	assert_eq!(
		next_dma_difficulty(
			1,
			vec![HeaderDifficultyInfo::from_ts_diff(42, hi.difficulty)]
		)
		.difficulty,
		Difficulty::from_num(14913)
	);

	// checking averaging works
	hi.difficulty = Difficulty::from_num(500);
	let sec = DMA_WINDOW / 2;
	let mut s1 = repeat(BLOCK_TIME_SEC, hi.clone(), sec, Some(cur_time));
	let mut s2 = repeat_offs(
		BLOCK_TIME_SEC,
		1500,
		sec,
		cur_time + (sec * BLOCK_TIME_SEC) as u64,
	);
	s2.append(&mut s1);
	assert_eq!(
		next_dma_difficulty(1, s2).difficulty,
		Difficulty::from_num(1000)
	);

	// too slow, diff goes down
	hi.difficulty = Difficulty::from_num(1000);
	assert_eq!(
		next_dma_difficulty(1, repeat(90, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(857)
	);
	assert_eq!(
		next_dma_difficulty(1, repeat(120, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(750)
	);

	// too fast, diff goes up
	assert_eq!(
		next_dma_difficulty(1, repeat(55, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(1028)
	);
	assert_eq!(
		next_dma_difficulty(1, repeat(45, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(1090)
	);
	assert_eq!(
		next_dma_difficulty(1, repeat(30, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(1200)
	);

	// hitting lower time bound, should always get the same result below
	assert_eq!(
		next_dma_difficulty(1, repeat(0, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(1500)
	);

	// hitting higher time bound, should always get the same result above
	assert_eq!(
		next_dma_difficulty(1, repeat(300, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(500)
	);
	assert_eq!(
		next_dma_difficulty(1, repeat(400, hi.clone(), just_enough, None)).difficulty,
		Difficulty::from_num(500)
	);

	// We should never drop below minimum
	hi.difficulty = Difficulty::zero();
	assert_eq!(
		next_dma_difficulty(1, repeat(90, hi, just_enough, None)).difficulty,
		Difficulty::min_dma()
	);
}

/// Checks different next_wtema_difficulty adjustments and difficulty boundaries
#[test]
fn next_wtema_difficulty_adjustment() {
	global::set_local_chain_type(global::ChainTypes::Mainnet);
	let hf4 = 2 * YEAR_HEIGHT; // height of HardFork4, switching to wtema DAA
	let diff_min = Difficulty::min_wtema();

	// Check we don't get stuck on mainnet difficulty <= Difficulty::min_wtema (on 59s blocks)
	let mut hi = HeaderDifficultyInfo::from_diff_scaling(diff_min, 0);
	hi.is_secondary = false;
	let hinext = next_wtema_difficulty(hf4, repeat(BLOCK_TIME_SEC - 1, hi.clone(), 2, None));

	assert_ne!(hinext.difficulty, diff_min);

	// 2 headers of data (last&prev), right interval, should stay constant
	let last2 = 2;
	hi.difficulty = Difficulty::from_num(20000);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(BLOCK_TIME_SEC, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(20000)
	);

	// too slow, diff goes down
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(61, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(19998)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(90, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(19958)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(120, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(19917)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(300, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(19672)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(400, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(19538)
	);

	// too fast, diff goes up
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(59, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(20001)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(55, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(20006)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(45, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(20020)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(30, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(20041)
	);
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(0, hi.clone(), last2, None)).difficulty,
		Difficulty::from_num(20083)
	);

	// We should never drop below minimum
	hi.difficulty = Difficulty::zero();
	assert_eq!(
		next_wtema_difficulty(hf4, repeat(90, hi, last2, None)).difficulty,
		Difficulty::min_wtema()
	);
}

// Builds an iterator for next difficulty calculation with the provided
// constant time interval, difficulty and total length.
fn repeat(
	interval: u64,
	diff: HeaderDifficultyInfo,
	len: u64,
	cur_time: Option<u64>,
) -> Vec<HeaderDifficultyInfo> {
	let cur_time = match cur_time {
		Some(t) => t,
		None => Utc::now().timestamp() as u64,
	};
	// watch overflow here, length shouldn't be ridiculous anyhow
	assert!(len < std::usize::MAX as u64);
	let diffs = vec![diff.difficulty; len as usize];
	let times = (0..(len as usize)).map(|n| n * interval as usize).rev();
	let pairs = times.zip(diffs.iter());
	pairs
		.map(|(t, d)| {
			HeaderDifficultyInfo::new(
				None,
				cur_time + t as u64,
				*d,
				diff.secondary_scaling,
				diff.is_secondary,
			)
		})
		.collect::<Vec<_>>()
}

fn repeat_offs(interval: u64, diff: u64, len: u64, from: u64) -> Vec<HeaderDifficultyInfo> {
	repeat(
		interval,
		HeaderDifficultyInfo::from_ts_diff(1, Difficulty::from_num(diff)),
		len,
		Some(from),
	)
}
