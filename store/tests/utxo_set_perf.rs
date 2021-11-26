// Copyright 2021 The Grin Developers
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

use env_logger;

use grin_store as store;

use chrono::prelude::Utc;
use std::fs;
use std::time::{Duration, Instant};

use croaring::Bitmap;

use crate::store::leaf_set::LeafSet;

pub fn as_millis(d: Duration) -> u128 {
	d.as_secs() as u128 * 1_000 as u128 + (d.subsec_nanos() / (1_000 * 1_000)) as u128
}

#[test]
fn test_leaf_set_performance() {
	let (mut leaf_set, data_dir) = setup("leaf_set_perf");

	println!("Timing some common operations:");

	// Add a million pos to the  set, syncing data to disk in 1,000 pos chunks
	// Simulating 1,000 blocks with 1,000 outputs each.
	let now = Instant::now();
	for x in 0..1_000 {
		for y in 0..1_000 {
			let pos = (x * 1_000) + y;
			leaf_set.add(pos);
		}
		leaf_set.flush().unwrap();
	}
	assert_eq!(leaf_set.len(), 1_000_000);
	println!(
		"Adding 1,000 chunks of 1,000 pos to leaf_set took {}ms",
		as_millis(now.elapsed())
	);

	// Simulate looking up existence of a large number of pos in the leaf_set.
	let now = Instant::now();
	for x in 0..1_000_000 {
		assert!(leaf_set.includes(x));
	}
	println!(
		"Checking 1,000,000 inclusions in leaf_set took {}ms",
		as_millis(now.elapsed())
	);

	// Remove a large number of pos in chunks to simulate blocks containing tx
	// spending outputs. Simulate 1,000 blocks each spending 1,000 outputs.
	let now = Instant::now();
	for x in 0..1_000 {
		for y in 0..1_000 {
			let pos = (x * 1_000) + y;
			leaf_set.remove(pos);
		}
		leaf_set.flush().unwrap();
	}
	assert_eq!(leaf_set.len(), 0);
	println!(
		"Removing 1,000 chunks of 1,000 pos from leaf_set took {}ms",
		as_millis(now.elapsed())
	);

	// Rewind pos in chunks of 1,000 to simulate rewinding over the same blocks.
	let now = Instant::now();
	for x in 0..1_000 {
		let from_pos = x * 1_000 + 1;
		let to_pos = from_pos + 1_000;
		let bitmap: Bitmap = (from_pos..to_pos).collect();
		leaf_set.rewind(1_000_000, &bitmap);
	}
	assert_eq!(leaf_set.len(), 1_000_000);
	println!(
		"Rewinding 1,000 chunks of 1,000 pos from leaf_set took {}ms",
		as_millis(now.elapsed())
	);

	// panic!("stop here to display results");

	teardown(data_dir);
}

fn setup(test_name: &str) -> (LeafSet, String) {
	let _ = env_logger::init();
	let data_dir = format!("./target/{}-{}", test_name, Utc::now().timestamp());
	fs::create_dir_all(data_dir.clone()).unwrap();
	let leaf_set = LeafSet::open(&format!("{}/{}", data_dir, "utxo.bin")).unwrap();
	(leaf_set, data_dir)
}

fn teardown(data_dir: String) {
	fs::remove_dir_all(data_dir).unwrap();
}
