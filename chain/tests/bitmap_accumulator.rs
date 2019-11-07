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
use self::core::core::hash;
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
	assert_eq!(accumulator.root(), hash::ZERO_HASH);

	// 1000... (rebuild from 0, setting [0] true)
	accumulator.apply(vec![0], vec![0]).unwrap();
	bit_vec.set(0, true);
	let expected_bytes = bit_vec.to_bytes();
	let expected_hash = expected_bytes.hash_with_index(0);
	assert_eq!(accumulator.root(), expected_hash);

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
}
