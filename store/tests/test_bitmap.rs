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

use rand;

use chrono::prelude::Utc;
use croaring::Bitmap;
use rand::Rng;

// We can use "andnot" to rewind easily by passing in a "bitmask" of
// all the subsequent pos we want to rewind.
#[test]
fn test_andnot_bitmap() {
	// bitmap:  10010011
	// bitmask: ....1111 (i.e. rewind to leave first 4 pos in place)
	// result:  1001....
	let bitmap: Bitmap = vec![1, 4, 7, 8].into_iter().collect();
	let bitmask: Bitmap = vec![5, 6, 7, 8].into_iter().collect();
	let res = bitmap.andnot(&bitmask);
	assert_eq!(res.to_vec(), vec![1, 4]);
}

// Alternatively we can use "and" to rewind easily by passing in a
// "bitmask" of all the pos we want to keep.
#[test]
fn test_and_bitmap() {
	// bitmap:  10010011
	// bitmask: 1111.... (i.e. rewind to leave first 4 pos in place)
	// result:  1001....
	let bitmap: Bitmap = vec![1, 4, 7, 8].into_iter().collect();
	let bitmask: Bitmap = vec![1, 2, 3, 4].into_iter().collect();
	let res = bitmap.and(&bitmask);
	assert_eq!(res.to_vec(), vec![1, 4]);
}

#[test]
fn test_flip_bitmap() {
	let bitmap: Bitmap = vec![1, 2, 4].into_iter().collect();
	let res = bitmap.flip(2..4);
	assert_eq!(res.to_vec(), vec![1, 3, 4]);
}

#[test]
fn test_a_small_bitmap() {
	let bitmap: Bitmap = vec![1, 99, 1_000].into_iter().collect();
	let serialized_buffer = bitmap.serialize();

	// we can store 3 pos in a roaring bitmap in 22 bytes
	// this is compared to storing them as a vec of u32 values which would be 4 * 3
	// = 12 bytes
	assert_eq!(serialized_buffer.len(), 22);
}

#[test]
fn test_1000_inputs() {
	let mut rng = rand::thread_rng();
	let mut bitmap = Bitmap::create();
	for _ in 1..1_000 {
		let n = rng.gen_range(0, 1_000_000);
		bitmap.add(n);
	}
	let serialized_buffer = bitmap.serialize();
	println!(
		"bitmap with 1,000 (out of 1,000,000) values in it: {}",
		serialized_buffer.len()
	);
	bitmap.run_optimize();
	let serialized_buffer = bitmap.serialize();
	println!(
		"bitmap with 1,000 (out of 1,000,000) values in it (optimized): {}",
		serialized_buffer.len()
	);
}

#[test]
fn test_a_big_bitmap() {
	let mut bitmap: Bitmap = (1..1_000_000).collect();
	let serialized_buffer = bitmap.serialize();

	// we can also store 1,000,000 pos in 131,208 bytes
	// a vec of u32s here would be 4,000,000 bytes
	assert_eq!(serialized_buffer.len(), 131_208);

	// but note we can optimize this heavily to get down to 230 bytes...
	assert!(bitmap.run_optimize());
	let serialized_buffer = bitmap.serialize();
	assert_eq!(serialized_buffer.len(), 230);
}

#[ignore]
#[test]
fn bench_fast_or() {
	let nano_to_millis = 1.0 / 1_000_000.0;

	let bitmaps_number = 256;
	let size_of_each_bitmap = 1_000;

	let init_bitmaps = || -> Vec<Bitmap> {
		let mut rng = rand::thread_rng();
		let mut bitmaps = vec![];
		for _ in 0..bitmaps_number {
			let mut bitmap = Bitmap::create();
			for _ in 0..size_of_each_bitmap {
				let n = rng.gen_range(0, 1_000_000);
				bitmap.add(n);
			}
			bitmaps.push(bitmap);
		}
		bitmaps
	};

	let mut bitmaps = init_bitmaps();
	let mut bitmap = Bitmap::create();
	let start = Utc::now().timestamp_nanos();
	for _ in 0..bitmaps_number {
		bitmap.or_inplace(&bitmaps.pop().unwrap());
	}
	let fin = Utc::now().timestamp_nanos();
	let dur_ms = (fin - start) as f64 * nano_to_millis;
	println!(
		"  or_inplace(): {:9.3?}ms. bitmap cardinality: {}",
		dur_ms,
		bitmap.cardinality()
	);

	let bitmaps = init_bitmaps();
	let start = Utc::now().timestamp_nanos();
	let bitmap = Bitmap::fast_or(&bitmaps.iter().map(|x| x).collect::<Vec<&Bitmap>>());
	let fin = Utc::now().timestamp_nanos();
	let dur_ms = (fin - start) as f64 * nano_to_millis;
	println!(
		"     fast_or(): {:9.3?}ms. bitmap cardinality: {}",
		dur_ms,
		bitmap.cardinality()
	);

	let bitmaps = init_bitmaps();
	let start = Utc::now().timestamp_nanos();
	let bitmap = Bitmap::fast_or_heap(&bitmaps.iter().map(|x| x).collect::<Vec<&Bitmap>>());
	let fin = Utc::now().timestamp_nanos();
	let dur_ms = (fin - start) as f64 * nano_to_millis;
	println!(
		"fast_or_heap(): {:9.3?}ms. bitmap cardinality: {}",
		dur_ms,
		bitmap.cardinality()
	);
}
