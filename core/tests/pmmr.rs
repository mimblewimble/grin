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

#[macro_use]
extern crate grin_core;

mod vec_backend;

use vec_backend::{TestElem, VecBackend};
use grin_core::core::hash::Hash;
use grin_core::core::pmmr::*;
use grin_core::ser::PMMRIndexHashable;

#[test]
fn some_all_ones() {
	for n in vec![1, 7, 255] {
		assert!(all_ones(n), "{} should be all ones", n);
	}
	for n in vec![0, 6, 9, 128] {
		assert!(!all_ones(n), "{} should not be all ones", n);
	}
}

#[test]
fn some_most_signif() {
	assert_eq!(most_significant_pos(0), 0);
	assert_eq!(most_significant_pos(1), 1);
	assert_eq!(most_significant_pos(6), 3);
	assert_eq!(most_significant_pos(7), 3);
	assert_eq!(most_significant_pos(8), 4);
	assert_eq!(most_significant_pos(128), 8);
}

#[test]
#[allow(unused_variables)]
fn first_100_mmr_heights() {
	let first_100_str = "0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 \
	                     0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 5 \
	                     0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 0 0 1 0 0";
	let first_100 = first_100_str.split(' ').map(|n| n.parse::<u64>().unwrap());
	let mut count = 1;
	for n in first_100 {
		assert_eq!(
			n,
			bintree_postorder_height(count),
			"expected {}, got {}",
			n,
			bintree_postorder_height(count)
		);
		count += 1;
	}
}

#[test]
fn test_n_leaves() {
	// make sure we handle an empty MMR correctly
	assert_eq!(n_leaves(0), 0);

	// and various sizes on non-empty MMRs
	assert_eq!(n_leaves(1), 1);
	assert_eq!(n_leaves(2), 2);
	assert_eq!(n_leaves(3), 2);
	assert_eq!(n_leaves(4), 3);
	assert_eq!(n_leaves(5), 4);
	assert_eq!(n_leaves(6), 4);
	assert_eq!(n_leaves(7), 4);
	assert_eq!(n_leaves(8), 5);
	assert_eq!(n_leaves(9), 6);
	assert_eq!(n_leaves(10), 6);
}

/// Find parent and sibling positions for various node positions.
#[test]
fn various_families() {
	// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3
	assert_eq!(family(1), (3, 2));
	assert_eq!(family(2), (3, 1));
	assert_eq!(family(3), (7, 6));
	assert_eq!(family(4), (6, 5));
	assert_eq!(family(5), (6, 4));
	assert_eq!(family(6), (7, 3));
	assert_eq!(family(7), (15, 14));
	assert_eq!(family(1_000), (1_001, 997));
}

#[test]
fn test_is_left_sibling() {
	assert_eq!(is_left_sibling(1), true);
	assert_eq!(is_left_sibling(2), false);
	assert_eq!(is_left_sibling(3), true);
}

#[test]
fn various_branches() {
	// the two leaf nodes in a 3 node tree (height 1)
	assert_eq!(family_branch(1, 3), [(3, 2)]);
	assert_eq!(family_branch(2, 3), [(3, 1)]);

	// the root node in a 3 node tree
	assert_eq!(family_branch(3, 3), []);

	// leaf node in a larger tree of 7 nodes (height 2)
	assert_eq!(family_branch(1, 7), [(3, 2), (7, 6)]);

	// note these only go as far up as the local peak, not necessarily the single
	// root
	assert_eq!(family_branch(1, 4), [(3, 2)]);
	// pos 4 in a tree of size 4 is a local peak
	assert_eq!(family_branch(4, 4), []);
	// pos 4 in a tree of size 5 is also still a local peak
	assert_eq!(family_branch(4, 5), []);
	// pos 4 in a tree of size 6 has a parent and a sibling
	assert_eq!(family_branch(4, 6), [(6, 5)]);
	// a tree of size 7 is all under a single root
	assert_eq!(family_branch(4, 7), [(6, 5), (7, 3)]);

	// ok now for a more realistic one, a tree with over a million nodes in it
	// find the "family path" back up the tree from a leaf node at 0
	// Note: the first two entries in the branch are consistent with a small 7 node
	// tree Note: each sibling is on the left branch, this is an example of the
	// largest possible list of peaks before we start combining them into larger
	// peaks.
	assert_eq!(
		family_branch(1, 1_049_000),
		[
			(3, 2),
			(7, 6),
			(15, 14),
			(31, 30),
			(63, 62),
			(127, 126),
			(255, 254),
			(511, 510),
			(1023, 1022),
			(2047, 2046),
			(4095, 4094),
			(8191, 8190),
			(16383, 16382),
			(32767, 32766),
			(65535, 65534),
			(131071, 131070),
			(262143, 262142),
			(524287, 524286),
			(1048575, 1048574),
		]
	);
}

#[test]
fn some_peaks() {
	// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3

	let empty: Vec<u64> = vec![];

	// make sure we handle an empty MMR correctly
	assert_eq!(peaks(0), empty);

	// and various non-empty MMRs
	assert_eq!(peaks(1), [1]);
	assert_eq!(peaks(2), empty);
	assert_eq!(peaks(3), [3]);
	assert_eq!(peaks(4), [3, 4]);
	assert_eq!(peaks(5), empty);
	assert_eq!(peaks(6), empty);
	assert_eq!(peaks(7), [7]);
	assert_eq!(peaks(8), [7, 8]);
	assert_eq!(peaks(9), empty);
	assert_eq!(peaks(10), [7, 10]);
	assert_eq!(peaks(11), [7, 10, 11]);
	assert_eq!(peaks(22), [15, 22]);
	assert_eq!(peaks(32), [31, 32]);
	assert_eq!(peaks(35), [31, 34, 35]);
	assert_eq!(peaks(42), [31, 38, 41, 42]);

	// large realistic example with almost 1.5 million nodes
	// note the distance between peaks decreases toward the right (trees get
	// smaller)
	assert_eq!(
		peaks(1048555),
		[
			524287, 786430, 917501, 983036, 1015803, 1032186, 1040377, 1044472, 1046519, 1047542,
			1048053, 1048308, 1048435, 1048498, 1048529, 1048544, 1048551, 1048554, 1048555,
		],
	);
}

#[test]
fn pmmr_bag_the_rhs() {
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

	pmmr.push(elems[0]).unwrap();
	let pos_0 = elems[0].hash_with_index(0);
	assert_eq!(pmmr.get_data(1), Some(elems[0]));

	pmmr.push(elems[1]).unwrap();
	let pos_1 = elems[1].hash_with_index(1);
	assert_eq!(pmmr.get_data(2), Some(elems[1]));
	let pos_2 = (pos_0, pos_1).hash_with_index(2);

	assert_eq!(pmmr.root(), pos_2);

	// only a single peak, so no rhs peaks to bag
	assert_eq!(pmmr.peaks(), vec![pos_2]);
	assert_eq!(pmmr.unpruned_size(), 3);
	assert_eq!(pmmr.bag_the_rhs(3), None);

	// push a third leaf, so we now have a couple of peaks to work with
	pmmr.push(elems[2]).unwrap();
	let pos_3 = elems[2].hash_with_index(3);
	assert_eq!(pmmr.peaks(), vec![pos_2, pos_3]);
	assert_eq!(pmmr.bag_the_rhs(3), Some(pos_3));

	// now push some more leaves to give us 3 peaks
	pmmr.push(elems[3]).unwrap();
	pmmr.push(elems[4]).unwrap();
	pmmr.push(elems[5]).unwrap();
	pmmr.push(elems[6]).unwrap();

	let peaks = pmmr.peaks();
	assert_eq!(peaks.len(), 3);
	assert_eq!(pmmr.unpruned_size(), 11);
	assert_eq!(
		pmmr.bag_the_rhs(7),
		Some((peaks[1], peaks[2]).hash_with_index(11))
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
	pmmr.push(elems[0]).unwrap();
	pmmr.dump(false);
	let pos_0 = elems[0].hash_with_index(0);
	assert_eq!(pmmr.peaks(), vec![pos_0]);
	assert_eq!(pmmr.root(), pos_0);
	assert_eq!(pmmr.unpruned_size(), 1);

	// two elements
	pmmr.push(elems[1]).unwrap();
	pmmr.dump(false);
	let pos_1 = elems[1].hash_with_index(1);
	let pos_2 = (pos_0, pos_1).hash_with_index(2);
	assert_eq!(pmmr.peaks(), vec![pos_2]);
	assert_eq!(pmmr.root(), pos_2);
	assert_eq!(pmmr.unpruned_size(), 3);

	// three elements
	pmmr.push(elems[2]).unwrap();
	pmmr.dump(false);
	let pos_3 = elems[2].hash_with_index(3);
	assert_eq!(pmmr.peaks(), vec![pos_2, pos_3]);
	assert_eq!(pmmr.root(), (pos_2, pos_3).hash_with_index(4));
	assert_eq!(pmmr.unpruned_size(), 4);

	// four elements
	pmmr.push(elems[3]).unwrap();
	pmmr.dump(false);
	let pos_4 = elems[3].hash_with_index(4);
	let pos_5 = (pos_3, pos_4).hash_with_index(5);
	let pos_6 = (pos_2, pos_5).hash_with_index(6);
	assert_eq!(pmmr.peaks(), vec![pos_6]);
	assert_eq!(pmmr.root(), pos_6);
	assert_eq!(pmmr.unpruned_size(), 7);

	// five elements
	pmmr.push(elems[4]).unwrap();
	pmmr.dump(false);
	let pos_7 = elems[4].hash_with_index(7);
	assert_eq!(pmmr.peaks(), vec![pos_6, pos_7]);
	assert_eq!(pmmr.root(), (pos_6, pos_7).hash_with_index(8));
	assert_eq!(pmmr.unpruned_size(), 8);

	// six elements
	pmmr.push(elems[5]).unwrap();
	let pos_8 = elems[5].hash_with_index(8);
	let pos_9 = (pos_7, pos_8).hash_with_index(9);
	assert_eq!(pmmr.peaks(), vec![pos_6, pos_9]);
	assert_eq!(pmmr.root(), (pos_6, pos_9).hash_with_index(10));
	assert_eq!(pmmr.unpruned_size(), 10);

	// seven elements
	pmmr.push(elems[6]).unwrap();
	let pos_10 = elems[6].hash_with_index(10);
	assert_eq!(pmmr.peaks(), vec![pos_6, pos_9, pos_10]);
	assert_eq!(
		pmmr.root(),
		(pos_6, (pos_9, pos_10).hash_with_index(11)).hash_with_index(11)
	);
	assert_eq!(pmmr.unpruned_size(), 11);

	// 001001200100123
	// eight elements
	pmmr.push(elems[7]).unwrap();
	let pos_11 = elems[7].hash_with_index(11);
	let pos_12 = (pos_10, pos_11).hash_with_index(12);
	let pos_13 = (pos_9, pos_12).hash_with_index(13);
	let pos_14 = (pos_6, pos_13).hash_with_index(14);
	assert_eq!(pmmr.peaks(), vec![pos_14]);
	assert_eq!(pmmr.root(), pos_14);
	assert_eq!(pmmr.unpruned_size(), 15);

	// nine elements
	pmmr.push(elems[8]).unwrap();
	let pos_15 = elems[8].hash_with_index(15);
	assert_eq!(pmmr.peaks(), vec![pos_14, pos_15]);
	assert_eq!(pmmr.root(), (pos_14, pos_15).hash_with_index(16));
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
	let res = pmmr.get_last_n_insertions(19);
	assert!(res.len() == 0);

	pmmr.push(elems[0]).unwrap();
	let res = pmmr.get_last_n_insertions(19);
	assert!(res.len() == 1);

	pmmr.push(elems[1]).unwrap();

	let res = pmmr.get_last_n_insertions(12);
	assert!(res.len() == 2);

	pmmr.push(elems[2]).unwrap();

	let res = pmmr.get_last_n_insertions(2);
	assert!(res.len() == 2);

	pmmr.push(elems[3]).unwrap();

	let res = pmmr.get_last_n_insertions(19);
	assert!(res.len() == 4);

	pmmr.push(elems[5]).unwrap();
	pmmr.push(elems[6]).unwrap();
	pmmr.push(elems[7]).unwrap();
	pmmr.push(elems[8]).unwrap();

	let res = pmmr.get_last_n_insertions(7);
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
			pmmr.push(*elem).unwrap();
		}
		orig_root = pmmr.root();
		sz = pmmr.unpruned_size();
	}

	// pruning a leaf with no parent should do nothing
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(16, 0).unwrap();
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 16);

	// pruning leaves with no shared parent just removes 1 element
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(2, 0).unwrap();
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 15);

	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(4, 0).unwrap();
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 14);

	// pruning a non-leaf node has no effect
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(3, 0).unwrap_err();
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 14);

	// pruning sibling removes subtree
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(5, 0).unwrap();
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 12);

	// pruning all leaves under level >1 removes all subtree
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		pmmr.prune(1, 0).unwrap();
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 9);

	// pruning everything should only leave us with a single peak
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
		for n in 1..16 {
			let _ = pmmr.prune(n, 0);
		}
		assert_eq!(orig_root, pmmr.root());
	}
	assert_eq!(ba.used_size(), 1);
}

#[test]
fn pmmr_next_pruned_idx() {
	let mut pl = PruneList::new();

	assert_eq!(pl.pruned_nodes.len(), 0);
	assert_eq!(pl.next_pruned_idx(1), Some(0));
	assert_eq!(pl.next_pruned_idx(2), Some(0));
	assert_eq!(pl.next_pruned_idx(3), Some(0));

	pl.add(2);
	assert_eq!(pl.pruned_nodes.len(), 1);
	assert_eq!(pl.pruned_nodes, [2]);
	assert_eq!(pl.next_pruned_idx(1), Some(0));
	assert_eq!(pl.next_pruned_idx(2), None);
	assert_eq!(pl.next_pruned_idx(3), Some(1));
	assert_eq!(pl.next_pruned_idx(4), Some(1));

	pl.add(1);
	assert_eq!(pl.pruned_nodes.len(), 1);
	assert_eq!(pl.pruned_nodes, [3]);
	assert_eq!(pl.next_pruned_idx(1), None);
	assert_eq!(pl.next_pruned_idx(2), None);
	assert_eq!(pl.next_pruned_idx(3), None);
	assert_eq!(pl.next_pruned_idx(4), Some(1));
	assert_eq!(pl.next_pruned_idx(5), Some(1));

	pl.add(3);
	assert_eq!(pl.pruned_nodes.len(), 1);
	assert_eq!(pl.pruned_nodes, [3]);
	assert_eq!(pl.next_pruned_idx(1), None);
	assert_eq!(pl.next_pruned_idx(2), None);
	assert_eq!(pl.next_pruned_idx(3), None);
	assert_eq!(pl.next_pruned_idx(4), Some(1));
	assert_eq!(pl.next_pruned_idx(5), Some(1));
}

#[test]
fn pmmr_prune_leaf_shift() {
	let mut pl = PruneList::new();

	// start with an empty prune list (nothing shifted)
	assert_eq!(pl.pruned_nodes.len(), 0);
	assert_eq!(pl.get_leaf_shift(1), Some(0));
	assert_eq!(pl.get_leaf_shift(2), Some(0));
	assert_eq!(pl.get_leaf_shift(4), Some(0));

	// now add a single leaf pos to the prune list
	// note this does not shift anything (we only start shifting after pruning a
	// parent)
	pl.add(1);
	assert_eq!(pl.pruned_nodes.len(), 1);
	assert_eq!(pl.pruned_nodes, [1]);
	assert_eq!(pl.get_leaf_shift(1), Some(0));
	assert_eq!(pl.get_leaf_shift(2), Some(0));
	assert_eq!(pl.get_leaf_shift(3), Some(0));
	assert_eq!(pl.get_leaf_shift(4), Some(0));

	// now add the sibling leaf pos (pos 1 and pos 2) which will prune the parent
	// at pos 3 this in turn will "leaf shift" the leaf at pos 3 by 2
	pl.add(2);
	assert_eq!(pl.pruned_nodes.len(), 1);
	assert_eq!(pl.pruned_nodes, [3]);
	assert_eq!(pl.get_leaf_shift(1), None);
	assert_eq!(pl.get_leaf_shift(2), None);
	assert_eq!(pl.get_leaf_shift(3), Some(2));
	assert_eq!(pl.get_leaf_shift(4), Some(2));
	assert_eq!(pl.get_leaf_shift(5), Some(2));

	// now prune an additional leaf at pos 4
	// leaf offset of subsequent pos will be 2
	// 00100120
	pl.add(4);
	assert_eq!(pl.pruned_nodes, [3, 4]);
	assert_eq!(pl.get_leaf_shift(1), None);
	assert_eq!(pl.get_leaf_shift(2), None);
	assert_eq!(pl.get_leaf_shift(3), Some(2));
	assert_eq!(pl.get_leaf_shift(4), Some(2));
	assert_eq!(pl.get_leaf_shift(5), Some(2));
	assert_eq!(pl.get_leaf_shift(6), Some(2));
	assert_eq!(pl.get_leaf_shift(7), Some(2));
	assert_eq!(pl.get_leaf_shift(8), Some(2));

	// now prune the sibling at pos 5
	// the two smaller subtrees (pos 3 and pos 6) are rolled up to larger subtree
	// (pos 7) the leaf offset is now 4 to cover entire subtree containing first
	// 4 leaves 00100120
	pl.add(5);
	assert_eq!(pl.pruned_nodes, [7]);
	assert_eq!(pl.get_leaf_shift(1), None);
	assert_eq!(pl.get_leaf_shift(2), None);
	assert_eq!(pl.get_leaf_shift(3), None);
	assert_eq!(pl.get_leaf_shift(4), None);
	assert_eq!(pl.get_leaf_shift(5), None);
	assert_eq!(pl.get_leaf_shift(6), None);
	assert_eq!(pl.get_leaf_shift(7), Some(4));
	assert_eq!(pl.get_leaf_shift(8), Some(4));
	assert_eq!(pl.get_leaf_shift(9), Some(4));

	// now check we can prune some of these in an arbitrary order
	// final result is one leaf (pos 2) and one small subtree (pos 6) pruned
	// with leaf offset of 2 to account for the pruned subtree
	let mut pl = PruneList::new();
	pl.add(2);
	pl.add(5);
	pl.add(4);
	assert_eq!(pl.pruned_nodes, [2, 6]);
	assert_eq!(pl.get_leaf_shift(1), Some(0));
	assert_eq!(pl.get_leaf_shift(2), Some(0));
	assert_eq!(pl.get_leaf_shift(3), Some(0));
	assert_eq!(pl.get_leaf_shift(4), None);
	assert_eq!(pl.get_leaf_shift(5), None);
	assert_eq!(pl.get_leaf_shift(6), Some(2));
	assert_eq!(pl.get_leaf_shift(7), Some(2));
	assert_eq!(pl.get_leaf_shift(8), Some(2));
	assert_eq!(pl.get_leaf_shift(9), Some(2));

	pl.add(1);
	assert_eq!(pl.pruned_nodes, [7]);
	assert_eq!(pl.get_leaf_shift(1), None);
	assert_eq!(pl.get_leaf_shift(2), None);
	assert_eq!(pl.get_leaf_shift(3), None);
	assert_eq!(pl.get_leaf_shift(4), None);
	assert_eq!(pl.get_leaf_shift(5), None);
	assert_eq!(pl.get_leaf_shift(6), None);
	assert_eq!(pl.get_leaf_shift(7), Some(4));
	assert_eq!(pl.get_leaf_shift(8), Some(4));
	assert_eq!(pl.get_leaf_shift(9), Some(4));
}

#[test]
fn pmmr_prune_shift() {
	let mut pl = PruneList::new();
	assert!(pl.pruned_nodes.is_empty());
	assert_eq!(pl.get_shift(1), Some(0));
	assert_eq!(pl.get_shift(2), Some(0));
	assert_eq!(pl.get_shift(3), Some(0));

	// prune a single leaf node
	// pruning only a leaf node does not shift any subsequent pos
	// we will only start shifting when a parent can be pruned
	pl.add(1);
	assert_eq!(pl.pruned_nodes, [1]);
	assert_eq!(pl.get_shift(1), Some(0));
	assert_eq!(pl.get_shift(2), Some(0));
	assert_eq!(pl.get_shift(3), Some(0));

	pl.add(2);
	assert_eq!(pl.pruned_nodes, [3]);
	assert_eq!(pl.get_shift(1), None);
	assert_eq!(pl.get_shift(2), None);
	// pos 3 is in the prune list, so removed but not compacted, but still shifted
	assert_eq!(pl.get_shift(3), Some(2));
	assert_eq!(pl.get_shift(4), Some(2));
	assert_eq!(pl.get_shift(5), Some(2));
	assert_eq!(pl.get_shift(6), Some(2));

	// pos 3 is not a leaf and is already in prune list
	// prune it and check we are still consistent
	pl.add(3);
	assert_eq!(pl.pruned_nodes, [3]);
	assert_eq!(pl.get_shift(1), None);
	assert_eq!(pl.get_shift(2), None);
	// pos 3 is in the prune list, so removed but not compacted, but still shifted
	assert_eq!(pl.get_shift(3), Some(2));
	assert_eq!(pl.get_shift(4), Some(2));
	assert_eq!(pl.get_shift(5), Some(2));
	assert_eq!(pl.get_shift(6), Some(2));

	pl.add(4);
	assert_eq!(pl.pruned_nodes, [3, 4]);
	assert_eq!(pl.get_shift(1), None);
	assert_eq!(pl.get_shift(2), None);
	// pos 3 is in the prune list, so removed but not compacted, but still shifted
	assert_eq!(pl.get_shift(3), Some(2));
	// pos 4 is also in the prune list and also shifted by same amount
	assert_eq!(pl.get_shift(4), Some(2));
	// subsequent nodes also shifted consistently
	assert_eq!(pl.get_shift(5), Some(2));
	assert_eq!(pl.get_shift(6), Some(2));

	pl.add(5);
	assert_eq!(pl.pruned_nodes, [7]);
	assert_eq!(pl.get_shift(1), None);
	assert_eq!(pl.get_shift(2), None);
	assert_eq!(pl.get_shift(3), None);
	assert_eq!(pl.get_shift(4), None);
	assert_eq!(pl.get_shift(5), None);
	assert_eq!(pl.get_shift(6), None);
	// everything prior to pos 7 is compacted away
	// pos 7 is shifted by 6 to account for this
	assert_eq!(pl.get_shift(7), Some(6));
	assert_eq!(pl.get_shift(8), Some(6));
	assert_eq!(pl.get_shift(9), Some(6));

	// prune a bunch more
	for x in 6..1000 {
		pl.add(x);
	}
	// and check we shift by a large number (hopefully the correct number...)
	assert_eq!(pl.get_shift(1010), Some(996));

	let mut pl = PruneList::new();
	pl.add(2);
	pl.add(5);
	pl.add(4);
	assert_eq!(pl.pruned_nodes, [2, 6]);
	assert_eq!(pl.get_shift(1), Some(0));
	assert_eq!(pl.get_shift(2), Some(0));
	assert_eq!(pl.get_shift(3), Some(0));
	assert_eq!(pl.get_shift(4), None);
	assert_eq!(pl.get_shift(5), None);
	assert_eq!(pl.get_shift(6), Some(2));
	assert_eq!(pl.get_shift(7), Some(2));
	assert_eq!(pl.get_shift(8), Some(2));
	assert_eq!(pl.get_shift(9), Some(2));

	// TODO - put some of these tests back in place for completeness

	//
	// let mut pl = PruneList::new();
	// pl.add(4);
	// assert_eq!(pl.pruned_nodes.len(), 1);
	// assert_eq!(pl.pruned_nodes, [4]);
	// assert_eq!(pl.get_shift(1), Some(0));
	// assert_eq!(pl.get_shift(2), Some(0));
	// assert_eq!(pl.get_shift(3), Some(0));
	// assert_eq!(pl.get_shift(4), None);
	// assert_eq!(pl.get_shift(5), Some(1));
	// assert_eq!(pl.get_shift(6), Some(1));
	//
	//
	// pl.add(5);
	// assert_eq!(pl.pruned_nodes.len(), 1);
	// assert_eq!(pl.pruned_nodes[0], 6);
	// assert_eq!(pl.get_shift(8), Some(3));
	// assert_eq!(pl.get_shift(2), Some(0));
	// assert_eq!(pl.get_shift(5), None);
	//
	// pl.add(2);
	// assert_eq!(pl.pruned_nodes.len(), 2);
	// assert_eq!(pl.pruned_nodes[0], 2);
	// assert_eq!(pl.get_shift(8), Some(4));
	// assert_eq!(pl.get_shift(1), Some(0));
	//
	// pl.add(8);
	// pl.add(11);
	// assert_eq!(pl.pruned_nodes.len(), 4);
	//
	// pl.add(1);
	// assert_eq!(pl.pruned_nodes.len(), 3);
	// assert_eq!(pl.pruned_nodes[0], 7);
	// assert_eq!(pl.get_shift(12), Some(9));
	//
	// pl.add(12);
	// assert_eq!(pl.pruned_nodes.len(), 3);
	// assert_eq!(pl.get_shift(12), None);
	// assert_eq!(pl.get_shift(9), Some(8));
	// assert_eq!(pl.get_shift(17), Some(11));
}

#[test]
fn check_all_ones() {
	for i in 0..1000000 {
		assert_eq!(old_all_ones(i), all_ones(i));
	}
}

// Check if the binary representation of a number is all ones.
fn old_all_ones(num: u64) -> bool {
	if num == 0 {
		return false;
	}
	let mut bit = 1;
	while num >= bit {
		if num & bit == 0 {
			return false;
		}
		bit = bit << 1;
	}
	true
}

#[test]
fn check_most_significant_pos() {
	for i in 0u64..1000000 {
		assert_eq!(old_most_significant_pos(i), most_significant_pos(i));
	}
}

// Get the position of the most significant bit in a number.
fn old_most_significant_pos(num: u64) -> u64 {
	let mut pos = 0;
	let mut bit = 1;
	while num >= bit {
		bit = bit << 1;
		pos += 1;
	}
	pos
}

#[test]
fn check_insertion_to_pmmr_index() {
	assert_eq!(insertion_to_pmmr_index(1), 1);
	assert_eq!(insertion_to_pmmr_index(2), 2);
	assert_eq!(insertion_to_pmmr_index(3), 4);
	assert_eq!(insertion_to_pmmr_index(4), 5);
	assert_eq!(insertion_to_pmmr_index(5), 8);
	assert_eq!(insertion_to_pmmr_index(6), 9);
	assert_eq!(insertion_to_pmmr_index(7), 11);
	assert_eq!(insertion_to_pmmr_index(8), 12);
}

#[test]
fn check_elements_from_insertion_index() {
	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);
	for x in 1..1000 {
		pmmr.push(TestElem([0, 0, 0, x])).unwrap();
	}
	// Normal case
	let res = pmmr.elements_from_insertion_index(1, 100);
	assert_eq!(res.0, 100);
	assert_eq!(res.1.len(), 100);
	assert_eq!(res.1[0].0[3], 1);
	assert_eq!(res.1[99].0[3], 100);

	// middle of pack
	let res = pmmr.elements_from_insertion_index(351, 70);
	assert_eq!(res.0, 420);
	assert_eq!(res.1.len(), 70);
	assert_eq!(res.1[0].0[3], 351);
	assert_eq!(res.1[69].0[3], 420);

	// past the end
	let res = pmmr.elements_from_insertion_index(650, 1000);
	assert_eq!(res.0, 999);
	assert_eq!(res.1.len(), 350);
	assert_eq!(res.1[0].0[3], 650);
	assert_eq!(res.1[349].0[3], 999);

	// pruning a few nodes should get consistent results
	pmmr.prune(insertion_to_pmmr_index(650), 0).unwrap();
	pmmr.prune(insertion_to_pmmr_index(651), 0).unwrap();
	pmmr.prune(insertion_to_pmmr_index(800), 0).unwrap();
	pmmr.prune(insertion_to_pmmr_index(900), 0).unwrap();
	pmmr.prune(insertion_to_pmmr_index(998), 0).unwrap();
	let res = pmmr.elements_from_insertion_index(650, 1000);
	assert_eq!(res.0, 999);
	assert_eq!(res.1.len(), 345);
	assert_eq!(res.1[0].0[3], 652);
	assert_eq!(res.1[344].0[3], 999);
}
