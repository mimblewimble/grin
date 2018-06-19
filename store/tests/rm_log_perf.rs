// Copyright 2018 The Grin Developers
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

extern crate croaring;
extern crate env_logger;
extern crate grin_core as core;
extern crate grin_store as store;
extern crate time;

use std::fs;
use std::time::{Duration, Instant};

use store::rm_log::RemoveLog;

pub fn as_millis(d: Duration) -> u128 {
	d.as_secs() as u128 * 1_000 as u128 + (d.subsec_nanos() / (1_000 * 1_000)) as u128
}

#[test]
fn test_rm_log_performance() {
	let (mut rm_log, data_dir) = setup("rm_log_perf");

	println!("Timing some common operations:");

	// Add a 1000 pos to the rm_log and sync to disk.
	let now = Instant::now();
	for x in 0..100 {
		for y in 0..1000 {
			let idx = x + 1;
			let pos = (x * 1000) + y + 1;
			rm_log.append(vec![pos], idx as u32).unwrap();
		}
		rm_log.flush().unwrap();
	}
	assert_eq!(rm_log.len(), 100_000);
	println!(
		"Adding 100 chunks of 1,000 pos to rm_log (syncing to disk) took {}ms",
		as_millis(now.elapsed())
	);

	// Add another 900,000 pos to the UTXO set, (do not sync each block, too
	// expensive)... Simulates 1,000 blocks with 1,000 outputs each.
	let now = Instant::now();
	for x in 100..1_000 {
		for y in 0..1_000 {
			let pos = (x * 1_000) + y + 1;
			rm_log.append(vec![pos], (x + 1) as u32).unwrap();
		}
		// Do not flush to disk each time (this gets very expensive).
		// rm_log.flush().unwrap();
	}
	// assert_eq!(rm_log.len(), 1_000_000);
	println!(
		"Adding 990 chunks of 1,000 pos to rm_log (without syncing) took {}ms",
		as_millis(now.elapsed())
	);

	// Simulate looking up existence of a large number of pos in the UTXO set.
	let now = Instant::now();
	for x in 0..1_000_000 {
		assert!(rm_log.includes(x + 1));
	}
	println!(
		"Checking 1,000,000 inclusions in rm_log took {}ms",
		as_millis(now.elapsed())
	);

	// Rewind pos in chunks of 1,000 to simulate rewinding over the same blocks.
	let now = Instant::now();
	let mut x = 1_000;
	while x > 0 {
		rm_log.rewind(x - 1).unwrap();
		x = x - 1;
	}
	rm_log.flush().unwrap();
	assert_eq!(rm_log.len(), 0);
	println!(
		"Rewinding 1,000 chunks of 1,000 pos from rm_log took {}ms",
		as_millis(now.elapsed())
	);

	// panic!("stop here to display results");

	teardown(data_dir);
}

fn setup(test_name: &str) -> (RemoveLog, String) {
	let _ = env_logger::init();
	let t = time::get_time();
	let data_dir = format!("./target/{}-{}", test_name, t.sec);
	fs::create_dir_all(data_dir.clone()).unwrap();
	let rm_log = RemoveLog::open(format!("{}/{}", data_dir, "rm_log.bin")).unwrap();
	(rm_log, data_dir)
}

fn teardown(data_dir: String) {
	fs::remove_dir_all(data_dir).unwrap();
}
