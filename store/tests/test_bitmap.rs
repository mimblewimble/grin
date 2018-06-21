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
extern crate rand;

use croaring::Bitmap;
use rand::Rng;

// We can use "andnot" to rewind the rm_log easily by passing in a "bitmask" of
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

// Alternatively we can use "and" to rewind the rm_log easily by passing in a
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
	// this is compared to storing them as a vec of u64 values which would be 8 * 3
	// = 32 bytes
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

	// we can also store 1,000 pos in 2,014 bytes
	// a vec of u64s here would be 8,000 bytes
	assert_eq!(serialized_buffer.len(), 131_208);

	// but note we can optimize this heavily to get down to 230 bytes...
	assert!(bitmap.run_optimize());
	let serialized_buffer = bitmap.serialize();
	assert_eq!(serialized_buffer.len(), 230);
}
