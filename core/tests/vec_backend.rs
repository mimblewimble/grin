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

mod common;

use self::core::core::pmmr::{ReadablePMMR, VecBackend, PMMR};
use crate::common::TestElem;
use grin_core as core;

#[test]
fn leaf_pos_and_idx_iter_test() {
	let elems = [
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
	];
	let mut backend = VecBackend::new();
	let mut pmmr = PMMR::new(&mut backend);
	for x in &elems {
		pmmr.push(x).unwrap();
	}
	assert_eq!(
		vec![0, 1, 2, 3, 4],
		pmmr.leaf_idx_iter(0).collect::<Vec<_>>()
	);
	assert_eq!(
		vec![0, 1, 3, 4, 7],
		pmmr.leaf_pos_iter().collect::<Vec<_>>()
	);
}

#[test]
fn leaf_pos_and_idx_iter_hash_only_test() {
	let elems = [
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
	];
	let mut backend = VecBackend::new_hash_only();
	let mut pmmr = PMMR::new(&mut backend);
	for x in &elems {
		pmmr.push(x).unwrap();
	}
	assert_eq!(
		vec![0, 1, 2, 3, 4],
		pmmr.leaf_idx_iter(0).collect::<Vec<_>>()
	);
	assert_eq!(
		vec![0, 1, 3, 4, 7],
		pmmr.leaf_pos_iter().collect::<Vec<_>>()
	);
}
