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

use self::core::core::hash::Hash;
use self::core::core::pmmr::{self, ReadablePMMR, VecBackend, PMMR};
use self::core::ser::PMMRIndexHashable;
use crate::common::TestElem;
use chrono::prelude::Utc;
use grin_core as core;

#[test]
fn some_peak_map() {
	assert_eq!(pmmr::peak_map_height(0), (0b0, 0));
	assert_eq!(pmmr::peak_map_height(1), (0b1, 0));
	assert_eq!(pmmr::peak_map_height(2), (0b1, 1));
	assert_eq!(pmmr::peak_map_height(3), (0b10, 0));
	assert_eq!(pmmr::peak_map_height(4), (0b11, 0));
	assert_eq!(pmmr::peak_map_height(5), (0b11, 1));
	assert_eq!(pmmr::peak_map_height(6), (0b11, 2));
	assert_eq!(pmmr::peak_map_height(7), (0b100, 0));
	assert_eq!(pmmr::peak_map_height(u64::MAX), ((u64::MAX >> 1) + 1, 0));
	assert_eq!(pmmr::peak_map_height(u64::MAX - 1), (u64::MAX >> 1, 63));
}

#[ignore]
#[test]
fn bench_peak_map() {
	let nano_to_millis = 1.0 / 1_000_000.0;

	let increments = vec![1_000_000u64, 10_000_000u64, 100_000_000u64];

	for v in increments {
		let start = Utc::now().timestamp_nanos();
		for i in 0..v {
			let _ = pmmr::peak_map_height(i);
		}
		let fin = Utc::now().timestamp_nanos();
		let dur_ms = (fin - start) as f64 * nano_to_millis;
		println!("{:9?} peak_map_height() in {:9.3?}ms", v, dur_ms);
	}
}

#[test]
fn some_peak_size() {
	assert_eq!(pmmr::peak_sizes_height(0), (vec![], 0));
	assert_eq!(pmmr::peak_sizes_height(1), (vec![1], 0));
	assert_eq!(pmmr::peak_sizes_height(2), (vec![1], 1));
	assert_eq!(pmmr::peak_sizes_height(3), (vec![3], 0));
	assert_eq!(pmmr::peak_sizes_height(4), (vec![3, 1], 0));
	assert_eq!(pmmr::peak_sizes_height(5), (vec![3, 1], 1));
	assert_eq!(pmmr::peak_sizes_height(6), (vec![3, 1], 2));
	assert_eq!(pmmr::peak_sizes_height(7), (vec![7], 0));
	assert_eq!(pmmr::peak_sizes_height(u64::MAX), (vec![u64::MAX], 0));

	let size_of_peaks = (1..64).map(|i| u64::MAX >> i).collect::<Vec<u64>>();
	assert_eq!(pmmr::peak_sizes_height(u64::MAX - 1), (size_of_peaks, 63));
}

#[test]
#[allow(unused_variables)]
fn first_100_mmr_heights() {
	let first_100_str = "0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 \
	                     0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 5 \
	                     0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 0 0 1 0 0";
	let first_100 = first_100_str.split(' ').map(|n| n.parse::<u64>().unwrap());
	let mut count = 0;
	for n in first_100 {
		assert_eq!(
			n,
			pmmr::bintree_postorder_height(count),
			"expected {}, got {}",
			n,
			pmmr::bintree_postorder_height(count)
		);
		count += 1;
	}
}

#[test]
fn test_bintree_range() {
	assert_eq!(pmmr::bintree_range(0), 0..1);
	assert_eq!(pmmr::bintree_range(1), 1..2);
	assert_eq!(pmmr::bintree_range(2), 0..3);
	assert_eq!(pmmr::bintree_range(3), 3..4);
	assert_eq!(pmmr::bintree_range(4), 4..5);
	assert_eq!(pmmr::bintree_range(5), 3..6);
	assert_eq!(pmmr::bintree_range(6), 0..7);
}

// The pos of the rightmost leaf for the provided MMR size (last leaf in subtree).
#[test]
fn test_bintree_rightmost() {
	assert_eq!(pmmr::bintree_rightmost(0), 0);
	assert_eq!(pmmr::bintree_rightmost(1), 1);
	assert_eq!(pmmr::bintree_rightmost(2), 1);
	assert_eq!(pmmr::bintree_rightmost(3), 3);
	assert_eq!(pmmr::bintree_rightmost(4), 4);
	assert_eq!(pmmr::bintree_rightmost(5), 4);
	assert_eq!(pmmr::bintree_rightmost(6), 4);
}

// The pos of the leftmost leaf for the provided MMR size (first leaf in subtree).
#[test]
fn test_bintree_leftmost() {
	assert_eq!(pmmr::bintree_leftmost(0), 0);
	assert_eq!(pmmr::bintree_leftmost(1), 1);
	assert_eq!(pmmr::bintree_leftmost(2), 0);
	assert_eq!(pmmr::bintree_leftmost(3), 3);
	assert_eq!(pmmr::bintree_leftmost(4), 4);
	assert_eq!(pmmr::bintree_leftmost(5), 3);
	assert_eq!(pmmr::bintree_leftmost(6), 0);
}

#[test]
fn test_bintree_leaf_pos_iter() {
	assert_eq!(pmmr::bintree_leaf_pos_iter(0).collect::<Vec<_>>(), [0]);
	assert_eq!(pmmr::bintree_leaf_pos_iter(1).collect::<Vec<_>>(), [1]);
	assert_eq!(pmmr::bintree_leaf_pos_iter(2).collect::<Vec<_>>(), [0, 1]);
	assert_eq!(pmmr::bintree_leaf_pos_iter(3).collect::<Vec<_>>(), [3]);
	assert_eq!(pmmr::bintree_leaf_pos_iter(4).collect::<Vec<_>>(), [4]);
	assert_eq!(pmmr::bintree_leaf_pos_iter(5).collect::<Vec<_>>(), [3, 4]);
	assert_eq!(
		pmmr::bintree_leaf_pos_iter(6).collect::<Vec<_>>(),
		[0, 1, 3, 4]
	);
}

#[test]
fn test_bintree_pos_iter() {
	assert_eq!(pmmr::bintree_pos_iter(0).collect::<Vec<_>>(), [0]);
	assert_eq!(pmmr::bintree_pos_iter(1).collect::<Vec<_>>(), [1]);
	assert_eq!(pmmr::bintree_pos_iter(2).collect::<Vec<_>>(), [0, 1, 2]);
	assert_eq!(pmmr::bintree_pos_iter(3).collect::<Vec<_>>(), [3]);
	assert_eq!(pmmr::bintree_pos_iter(4).collect::<Vec<_>>(), [4]);
	assert_eq!(pmmr::bintree_pos_iter(5).collect::<Vec<_>>(), [3, 4, 5]);
	assert_eq!(
		pmmr::bintree_pos_iter(6).collect::<Vec<_>>(),
		[0, 1, 2, 3, 4, 5, 6]
	);
}

#[test]
fn test_is_leaf() {
	assert_eq!(pmmr::is_leaf(0), true);
	assert_eq!(pmmr::is_leaf(1), true);
	assert_eq!(pmmr::is_leaf(2), false);
	assert_eq!(pmmr::is_leaf(3), true);
	assert_eq!(pmmr::is_leaf(4), true);
	assert_eq!(pmmr::is_leaf(5), false);
	assert_eq!(pmmr::is_leaf(6), false);
}

#[test]
fn test_pmmr_leaf_to_insertion_index() {
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(0), Some(0));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(1), Some(1));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(3), Some(2));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(4), Some(3));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(7), Some(4));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(8), Some(5));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(10), Some(6));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(11), Some(7));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(15), Some(8));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(16), Some(9));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(18), Some(10));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(19), Some(11));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(22), Some(12));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(23), Some(13));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(25), Some(14));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(26), Some(15));
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(31), Some(16));

	// Not a leaf node
	assert_eq!(pmmr::pmmr_leaf_to_insertion_index(30), None);

	// Sanity check to make sure we don't get an explosion around the u64 max
	// number of leaves
	let n_leaves_max_u64 = pmmr::n_leaves(u64::MAX - 257);
	assert_eq!(
		pmmr::pmmr_leaf_to_insertion_index(n_leaves_max_u64),
		Some(4611686018427387884)
	);
}

#[test]
fn test_n_leaves() {
	// make sure we handle an empty MMR correctly
	assert_eq!(pmmr::n_leaves(0), 0);
	// and various sizes on non-empty MMRs
	assert_eq!(pmmr::n_leaves(1), 1);
	assert_eq!(pmmr::n_leaves(2), 2);
	assert_eq!(pmmr::n_leaves(3), 2);
	assert_eq!(pmmr::n_leaves(4), 3);
	assert_eq!(pmmr::n_leaves(5), 4);
	assert_eq!(pmmr::n_leaves(6), 4);
	assert_eq!(pmmr::n_leaves(7), 4);
	assert_eq!(pmmr::n_leaves(8), 5);
	assert_eq!(pmmr::n_leaves(9), 6);
	assert_eq!(pmmr::n_leaves(10), 6);
}

#[test]
fn test_round_up_to_leaf_pos() {
	assert_eq!(pmmr::round_up_to_leaf_pos(0), 0);
	assert_eq!(pmmr::round_up_to_leaf_pos(1), 1);
	assert_eq!(pmmr::round_up_to_leaf_pos(2), 3);
	assert_eq!(pmmr::round_up_to_leaf_pos(3), 3);
	assert_eq!(pmmr::round_up_to_leaf_pos(4), 4);
	assert_eq!(pmmr::round_up_to_leaf_pos(5), 7);
	assert_eq!(pmmr::round_up_to_leaf_pos(6), 7);
	assert_eq!(pmmr::round_up_to_leaf_pos(7), 7);
	assert_eq!(pmmr::round_up_to_leaf_pos(8), 8);
	assert_eq!(pmmr::round_up_to_leaf_pos(9), 10);
	assert_eq!(pmmr::round_up_to_leaf_pos(10), 10);
}

/// Find parent and sibling positions for various node positions.
#[test]
fn various_families() {
	// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3
	assert_eq!(pmmr::family(0), (2, 1));
	assert_eq!(pmmr::family(1), (2, 0));
	assert_eq!(pmmr::family(2), (6, 5));
	assert_eq!(pmmr::family(3), (5, 4));
	assert_eq!(pmmr::family(4), (5, 3));
	assert_eq!(pmmr::family(5), (6, 2));
	assert_eq!(pmmr::family(6), (14, 13));
	assert_eq!(pmmr::family(999), (1_000, 996));
}

#[test]
fn test_is_left_sibling() {
	assert_eq!(pmmr::is_left_sibling(0), true);
	assert_eq!(pmmr::is_left_sibling(1), false);
	assert_eq!(pmmr::is_left_sibling(2), true);
}

#[test]
fn various_branches() {
	// the two leaf nodes in a 3 node tree (height 1)
	assert_eq!(pmmr::family_branch(0, 3), [(2, 1)]);
	assert_eq!(pmmr::family_branch(1, 3), [(2, 0)]);

	// the root node in a 3 node tree
	assert_eq!(pmmr::family_branch(2, 3), []);

	// leaf node in a larger tree of 7 nodes (height 2)
	assert_eq!(pmmr::family_branch(0, 7), [(2, 1), (6, 5)]);

	// note these only go as far up as the local peak, not necessarily the single
	// root
	assert_eq!(pmmr::family_branch(0, 4), [(2, 1)]);
	// pos 4 in a tree of size 4 is a local peak
	assert_eq!(pmmr::family_branch(3, 4), []);
	// pos 4 in a tree of size 5 is also still a local peak
	assert_eq!(pmmr::family_branch(3, 5), []);
	// pos 4 in a tree of size 6 has a parent and a sibling
	assert_eq!(pmmr::family_branch(3, 6), [(5, 4)]);
	// a tree of size 7 is all under a single root
	assert_eq!(pmmr::family_branch(3, 7), [(5, 4), (6, 2)]);

	// ok now for a more realistic one, a tree with over a million nodes in it
	// find the "family path" back up the tree from a leaf node at 0
	// Note: the first two entries in the branch are consistent with a small 7 node
	// tree Note: each sibling is on the left branch, this is an example of the
	// largest possible list of peaks before we start combining them into larger
	// peaks.
	assert_eq!(
		pmmr::family_branch(0, 1_049_000),
		[
			(2, 1),
			(6, 5),
			(14, 13),
			(30, 29),
			(62, 61),
			(126, 125),
			(254, 253),
			(510, 509),
			(1022, 1021),
			(2046, 2045),
			(4094, 4093),
			(8190, 8189),
			(16382, 16381),
			(32766, 32765),
			(65534, 65533),
			(131070, 131069),
			(262142, 262141),
			(524286, 524285),
			(1048574, 1048573),
		]
	);
}

#[test]
fn some_peaks() {
	// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3

	let empty: Vec<u64> = vec![];

	// make sure we handle an empty MMR correctly
	assert_eq!(pmmr::peaks(0), empty);

	// and various non-empty MMRs
	assert_eq!(pmmr::peaks(1), [0]);
	assert_eq!(pmmr::peaks(2), empty);
	assert_eq!(pmmr::peaks(3), [2]);
	assert_eq!(pmmr::peaks(4), [2, 3]);
	assert_eq!(pmmr::peaks(5), empty);
	assert_eq!(pmmr::peaks(6), empty);
	assert_eq!(pmmr::peaks(7), [6]);
	assert_eq!(pmmr::peaks(8), [6, 7]);
	assert_eq!(pmmr::peaks(9), empty);
	assert_eq!(pmmr::peaks(10), [6, 9]);
	assert_eq!(pmmr::peaks(11), [6, 9, 10]);
	assert_eq!(pmmr::peaks(22), [14, 21]);
	assert_eq!(pmmr::peaks(32), [30, 31]);
	assert_eq!(pmmr::peaks(35), [30, 33, 34]);
	assert_eq!(pmmr::peaks(42), [30, 37, 40, 41]);

	// large realistic example with almost 1.5 million nodes
	// note the distance between peaks decreases toward the right (trees get
	// smaller)
	assert_eq!(
		pmmr::peaks(1048555),
		[
			524286, 786429, 917500, 983035, 1015802, 1032185, 1040376, 1044471, 1046518, 1047541,
			1048052, 1048307, 1048434, 1048497, 1048528, 1048543, 1048550, 1048553, 1048554,
		],
	);
}

#[test]
#[allow(unused_variables)]
fn pmmr_push_root() {
	let elems = [
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
		TestElem([1, 0, 0, 0]),
	];

	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);

	// one element
	pmmr.push(&elems[0]).unwrap();
	pmmr.dump(false);
	let pos_0 = elems[0].hash_with_index(0);
	assert_eq!(pmmr.peaks(), vec![pos_0]);
	assert_eq!(pmmr.root().unwrap(), pos_0);
	assert_eq!(pmmr.unpruned_size(), 1);

	// two elements
	pmmr.push(&elems[1]).unwrap();
	pmmr.dump(false);
	let pos_1 = elems[1].hash_with_index(1);
	let pos_2 = (pos_0, pos_1).hash_with_index(2);
	assert_eq!(pmmr.peaks(), vec![pos_2]);
	assert_eq!(pmmr.root().unwrap(), pos_2);
	assert_eq!(pmmr.unpruned_size(), 3);

	// three elements
	pmmr.push(&elems[2]).unwrap();
	pmmr.dump(false);
	let pos_3 = elems[2].hash_with_index(3);
	assert_eq!(pmmr.peaks(), vec![pos_2, pos_3]);
	assert_eq!(pmmr.root().unwrap(), (pos_2, pos_3).hash_with_index(4));
	assert_eq!(pmmr.unpruned_size(), 4);

	// four elements
	pmmr.push(&elems[3]).unwrap();
	pmmr.dump(false);
	let pos_4 = elems[3].hash_with_index(4);
	let pos_5 = (pos_3, pos_4).hash_with_index(5);
	let pos_6 = (pos_2, pos_5).hash_with_index(6);
	assert_eq!(pmmr.peaks(), vec![pos_6]);
	assert_eq!(pmmr.root().unwrap(), pos_6);
	assert_eq!(pmmr.unpruned_size(), 7);

	// five elements
	pmmr.push(&elems[4]).unwrap();
	pmmr.dump(false);
	let pos_7 = elems[4].hash_with_index(7);
	assert_eq!(pmmr.peaks(), vec![pos_6, pos_7]);
	assert_eq!(pmmr.root().unwrap(), (pos_6, pos_7).hash_with_index(8));
	assert_eq!(pmmr.unpruned_size(), 8);

	// six elements
	pmmr.push(&elems[5]).unwrap();
	let pos_8 = elems[5].hash_with_index(8);
	let pos_9 = (pos_7, pos_8).hash_with_index(9);
	assert_eq!(pmmr.peaks(), vec![pos_6, pos_9]);
	assert_eq!(pmmr.root().unwrap(), (pos_6, pos_9).hash_with_index(10));
	assert_eq!(pmmr.unpruned_size(), 10);

	// seven elements
	pmmr.push(&elems[6]).unwrap();
	let pos_10 = elems[6].hash_with_index(10);
	assert_eq!(pmmr.peaks(), vec![pos_6, pos_9, pos_10]);
	assert_eq!(
		pmmr.root().unwrap(),
		(pos_6, (pos_9, pos_10).hash_with_index(11)).hash_with_index(11)
	);
	assert_eq!(pmmr.unpruned_size(), 11);

	// 001001200100123
	// eight elements
	pmmr.push(&elems[7]).unwrap();
	let pos_11 = elems[7].hash_with_index(11);
	let pos_12 = (pos_10, pos_11).hash_with_index(12);
	let pos_13 = (pos_9, pos_12).hash_with_index(13);
	let pos_14 = (pos_6, pos_13).hash_with_index(14);
	assert_eq!(pmmr.peaks(), vec![pos_14]);
	assert_eq!(pmmr.root().unwrap(), pos_14);
	assert_eq!(pmmr.unpruned_size(), 15);

	// nine elements
	pmmr.push(&elems[8]).unwrap();
	let pos_15 = elems[8].hash_with_index(15);
	assert_eq!(pmmr.peaks(), vec![pos_14, pos_15]);
	assert_eq!(pmmr.root().unwrap(), (pos_14, pos_15).hash_with_index(16));
	assert_eq!(pmmr.unpruned_size(), 16);
}

#[test]
fn pmmr_get_last_n_insertions() {
	let elems = [
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
		TestElem([1, 0, 0, 0]),
	];

	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);

	// test when empty
	let res = pmmr.readonly_pmmr().get_last_n_insertions(19);
	assert!(res.is_empty());

	pmmr.push(&elems[0]).unwrap();
	let res = pmmr.readonly_pmmr().get_last_n_insertions(19);
	assert!(res.len() == 1);

	pmmr.push(&elems[1]).unwrap();

	let res = pmmr.readonly_pmmr().get_last_n_insertions(12);
	assert!(res.len() == 2);

	pmmr.push(&elems[2]).unwrap();

	let res = pmmr.readonly_pmmr().get_last_n_insertions(2);
	assert!(res.len() == 2);

	pmmr.push(&elems[3]).unwrap();

	let res = pmmr.readonly_pmmr().get_last_n_insertions(19);
	assert!(res.len() == 4);

	pmmr.push(&elems[5]).unwrap();
	pmmr.push(&elems[6]).unwrap();
	pmmr.push(&elems[7]).unwrap();
	pmmr.push(&elems[8]).unwrap();

	let res = pmmr.readonly_pmmr().get_last_n_insertions(7);
	assert!(res.len() == 7);
}

#[test]
#[allow(unused_variables)]
fn pmmr_prune() {
	let elems = [
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
		TestElem([1, 0, 0, 0]),
	];

	let orig_root: Hash;
	let sz: u64;
	let mut ba = VecBackend::new();
	{
		let mut pmmr = PMMR::new(&mut ba);
		for elem in &elems[..] {
			pmmr.push(elem).unwrap();
		}
		orig_root = pmmr.root().unwrap();
		sz = pmmr.unpruned_size();
	}

	// First check the initial numbers of elements.
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 0);

	// pruning a leaf with no parent should do nothing
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(15).unwrap();
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 1);

	// pruning leaves with no shared parent just removes 1 element
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(1).unwrap();
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 2);

	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(3).unwrap();
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 3);

	// pruning a non-leaf node has no effect
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(2).unwrap_err();
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 3);

	// TODO - no longer true (leaves only now) - pruning sibling removes subtree
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(4).unwrap();
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 4);

	// TODO - no longer true (leaves only now) - pruning all leaves under level >1
	// removes all subtree
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(0).unwrap();
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 5);

	// pruning everything should only leave us with a single peak
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut ba, sz);
		for n in 0..15 {
			let _ = pmmr.prune(n);
		}
		assert_eq!(orig_root, pmmr.root().unwrap());
	}
	assert_eq!(ba.hashes.len(), 16);
	assert_eq!(ba.removed.len(), 9);
}

#[test]
fn check_insertion_to_pmmr_index() {
	assert_eq!(pmmr::insertion_to_pmmr_index(0), 0);
	assert_eq!(pmmr::insertion_to_pmmr_index(1), 1);
	assert_eq!(pmmr::insertion_to_pmmr_index(2), 3);
	assert_eq!(pmmr::insertion_to_pmmr_index(3), 4);
	assert_eq!(pmmr::insertion_to_pmmr_index(4), 7);
	assert_eq!(pmmr::insertion_to_pmmr_index(5), 8);
	assert_eq!(pmmr::insertion_to_pmmr_index(6), 10);
	assert_eq!(pmmr::insertion_to_pmmr_index(7), 11);
}

#[test]
fn check_elements_from_pmmr_index() {
	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);
	// 20 elements should give max index 38
	for x in 1..21 {
		pmmr.push(&TestElem([0, 0, 0, x])).unwrap();
	}

	// Normal case
	let res = pmmr.readonly_pmmr().elements_from_pmmr_index(1, 1000, None);
	assert_eq!(res.0, 38);
	assert_eq!(res.1.len(), 20);
	assert_eq!(res.1[0].0[3], 1);
	assert_eq!(res.1[19].0[3], 20);

	// middle of pack
	let res = pmmr
		.readonly_pmmr()
		.elements_from_pmmr_index(8, 1000, Some(34));
	assert_eq!(res.0, 34);
	assert_eq!(res.1.len(), 14);
	assert_eq!(res.1[0].0[3], 5);
	assert_eq!(res.1[13].0[3], 18);

	// bounded
	let res = pmmr
		.readonly_pmmr()
		.elements_from_pmmr_index(8, 7, Some(34));
	assert_eq!(res.0, 19);
	assert_eq!(res.1.len(), 7);
	assert_eq!(res.1[0].0[3], 5);
	assert_eq!(res.1[6].0[3], 11);

	// pruning a few nodes should get consistent results
	pmmr.prune(pmmr::insertion_to_pmmr_index(4)).unwrap();
	pmmr.prune(pmmr::insertion_to_pmmr_index(19)).unwrap();

	let res = pmmr
		.readonly_pmmr()
		.elements_from_pmmr_index(8, 7, Some(34));
	assert_eq!(res.0, 20);
	assert_eq!(res.1.len(), 7);
	assert_eq!(res.1[0].0[3], 6);
	assert_eq!(res.1[6].0[3], 12);
}
