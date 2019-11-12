// Copyright 2019 The Grin Developers
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

use self::chain::txhashset::BitmapAccumulator;
use self::core::core::hash::Hash;
use self::core::ser::PMMRIndexHashable;
use bit_vec::BitVec;
use grin_chain as chain;
use grin_core as core;
use grin_util as util;

#[test]
fn test_bitmap_accumulator() {
	util::init_test_logger();

	let mut bit_vec = BitVec::from_elem(1024, false);
	let mut accumulator = BitmapAccumulator::new();
	assert_eq!(accumulator.root(), Hash::default());

	// 1000... (rebuild from 0, setting [0] true)
	accumulator.apply(vec![0], vec![0]).unwrap();
	bit_vec.set(0, true);
	let expected_bytes = bit_vec.to_bytes();
	let expected_hash = expected_bytes.hash_with_index(0);
	assert_eq!(accumulator.root(), expected_hash);

	// Check that removing the last bit in a chunk removes the now empty chunk
	// if it is the rightmost chunk.
	accumulator.apply(vec![0], vec![]).unwrap();
	assert_eq!(accumulator.root(), Hash::default());

	// 1100... (rebuild from 0, setting [0, 1] true)
	accumulator.apply(vec![0], vec![0, 1]).unwrap();
	bit_vec.set(1, true);
	let expected_bytes = bit_vec.to_bytes();
	let expected_hash = expected_bytes.hash_with_index(0);
	assert_eq!(accumulator.root(), expected_hash);

	// 0100... (rebuild from 0, setting [1] true, which will reset [0] false)
	accumulator.apply(vec![0], vec![1]).unwrap();
	bit_vec.set(0, false);
	let expected_bytes = bit_vec.to_bytes();
	let expected_hash = expected_bytes.hash_with_index(0);
	assert_eq!(accumulator.root(), expected_hash);

	// 0100... (rebuild from 1, setting [1] true)
	accumulator.apply(vec![1], vec![1]).unwrap();
	let expected_bytes = bit_vec.to_bytes();
	let expected_hash = expected_bytes.hash_with_index(0);
	assert_eq!(accumulator.root(), expected_hash);

	// 0100...0001 (rebuild from 0, setting [1, 1023] true)
	accumulator.apply(vec![0], vec![1, 1023]).unwrap();
	bit_vec.set(1023, true);
	let expected_bytes = bit_vec.to_bytes();
	let expected_hash = expected_bytes.hash_with_index(0);
	assert_eq!(accumulator.root(), expected_hash);

	// Now set bits such that we extend the bitmap accumulator across multiple 1024 bit chunks.
	// We need a second bit_vec here to reflect the additional chunk.
	// 0100...0001, 1000...0000 (rebuild from 0, setting [1, 1023, 1024] true)
	accumulator.apply(vec![0], vec![1, 1023, 1024]).unwrap();
	let mut bit_vec2 = BitVec::from_elem(1024, false);
	bit_vec2.set(0, true);
	let expected_hash = {
		let expected_bytes_0 = bit_vec.to_bytes();
		let expected_bytes_1 = bit_vec2.to_bytes();
		let expected_hash_0 = expected_bytes_0.hash_with_index(0);
		let expected_hash_1 = expected_bytes_1.hash_with_index(1);
		(expected_hash_0, expected_hash_1).hash_with_index(2)
	};
	assert_eq!(accumulator.root(), expected_hash);

	// Just rebuild the second bitmap chunk.
	// 0100...0001, 0100...0000 (rebuild from 1025, setting [1025] true)
	accumulator.apply(vec![1025], vec![1025]).unwrap();
	bit_vec2.set(0, false);
	bit_vec2.set(1, true);
	let expected_hash = {
		let expected_bytes_0 = bit_vec.to_bytes();
		let expected_bytes_1 = bit_vec2.to_bytes();
		let expected_hash_0 = expected_bytes_0.hash_with_index(0);
		let expected_hash_1 = expected_bytes_1.hash_with_index(1);
		(expected_hash_0, expected_hash_1).hash_with_index(2)
	};
	assert_eq!(accumulator.root(), expected_hash);

	// Rebuild the first bitmap chunk and all chunks after it.
	// 0100...0000, 0100...0000 (rebuild from 1, setting [1, 1025] true)
	accumulator.apply(vec![1], vec![1, 1025]).unwrap();
	bit_vec.set(1023, false);
	let expected_hash = {
		let expected_bytes_0 = bit_vec.to_bytes();
		let expected_bytes_1 = bit_vec2.to_bytes();
		let expected_hash_0 = expected_bytes_0.hash_with_index(0);
		let expected_hash_1 = expected_bytes_1.hash_with_index(1);
		(expected_hash_0, expected_hash_1).hash_with_index(2)
	};
	assert_eq!(accumulator.root(), expected_hash);

	// Make sure we handle the case where the first chunk is all 0s
	// 0000...0000, 0100...0000 (rebuild from 1, setting [1025] true)
	accumulator.apply(vec![1], vec![1025]).unwrap();
	bit_vec.set(1, false);
	let expected_hash = {
		let expected_bytes_0 = bit_vec.to_bytes();
		let expected_bytes_1 = bit_vec2.to_bytes();
		let expected_hash_0 = expected_bytes_0.hash_with_index(0);
		let expected_hash_1 = expected_bytes_1.hash_with_index(1);
		(expected_hash_0, expected_hash_1).hash_with_index(2)
	};
	assert_eq!(accumulator.root(), expected_hash);

	// Make sure we handle the case where the all chunks are all 0s.
	// Here we trim all the "empty" chunks leaving an empty accumulator.
	// 0000...0000, 0000...0000 (rebuild from 1025, setting [] true)
	accumulator.apply(vec![1025], vec![]).unwrap();
	assert_eq!(accumulator.root(), Hash::default());

	// Make sure we pad appropriately with 0s if we set a distant bit to 1.
	// 0000...0000, 0100...0000 (rebuild from 1025, setting [1025] true)
	accumulator.apply(vec![1025], vec![1025]).unwrap();
	let expected_hash = {
		let expected_bytes_0 = bit_vec.to_bytes();
		let expected_bytes_1 = bit_vec2.to_bytes();
		let expected_hash_0 = expected_bytes_0.hash_with_index(0);
		let expected_hash_1 = expected_bytes_1.hash_with_index(1);
		(expected_hash_0, expected_hash_1).hash_with_index(2)
	};
	assert_eq!(accumulator.root(), expected_hash);
}
