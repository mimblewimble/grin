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

use grin_store as store;

use crate::store::prune_list::PruneList;

#[test]
fn test_is_pruned() {
	let mut pl = PruneList::empty();

	assert_eq!(pl.len(), 0);
	assert_eq!(pl.is_pruned(0), false);
	assert_eq!(pl.is_pruned(1), false);
	assert_eq!(pl.is_pruned(2), false);

	pl.append(1);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [2]);
	assert_eq!(pl.is_pruned(0), false);
	assert_eq!(pl.is_pruned(1), true);
	assert_eq!(pl.is_pruned(2), false);
	assert_eq!(pl.is_pruned(3), false);

	let mut pl = PruneList::empty();
	pl.append(0);
	pl.append(1);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 1);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [3]);
	assert_eq!(pl.is_pruned(0), true);
	assert_eq!(pl.is_pruned(1), true);
	assert_eq!(pl.is_pruned(2), true);
	assert_eq!(pl.is_pruned(3), false);

	pl.append(3);

	// Flushing the prune_list removes any individual leaf positions.
	// This assumes we will track these outside the prune_list via the leaf_set.
	pl.flush().unwrap();

	assert_eq!(pl.len(), 2);
	assert_eq!(pl.to_vec(), [3, 4]);
	assert_eq!(pl.is_pruned(0), true);
	assert_eq!(pl.is_pruned(1), true);
	assert_eq!(pl.is_pruned(2), true);
	assert_eq!(pl.is_pruned(3), true);
	assert_eq!(pl.is_pruned(4), false);
}

#[test]
fn test_get_leaf_shift() {
	let mut pl = PruneList::empty();

	// start with an empty prune list (nothing shifted)
	assert_eq!(pl.len(), 0);
	assert_eq!(pl.get_leaf_shift(4), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);

	// now add a single leaf pos to the prune list
	// leaves will not shift shift anything
	// we only start shifting after pruning a parent
	pl.append(0);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [1]);
	assert_eq!(pl.get_leaf_shift(0), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);

	// now add the sibling leaf pos (pos 1) which will prune the parent
	// at pos 2 this in turn will "leaf shift" the leaf at pos 2 by 2
	pl.append(1);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 1);
	assert_eq!(pl.get_leaf_shift(0), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 2);
	assert_eq!(pl.get_leaf_shift(3), 2);
	assert_eq!(pl.get_leaf_shift(4), 2);

	// now prune an additional leaf at pos 3
	// leaf offset of subsequent pos will be 2
	// 00100120
	pl.append(3);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 2);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [3, 4]);
	assert_eq!(pl.get_leaf_shift(0), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 2);
	assert_eq!(pl.get_leaf_shift(3), 2);
	assert_eq!(pl.get_leaf_shift(4), 2);
	assert_eq!(pl.get_leaf_shift(5), 2);
	assert_eq!(pl.get_leaf_shift(6), 2);
	assert_eq!(pl.get_leaf_shift(7), 2);

	// now prune the sibling at pos 4
	// the two smaller subtrees (pos 2 and pos 5) are rolled up to larger subtree
	// (pos 6) the leaf offset is now 4 to cover entire subtree containing first
	// 4 leaves 00100120
	pl.append(4);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 1);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [7]);
	assert_eq!(pl.get_leaf_shift(0), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);
	assert_eq!(pl.get_leaf_shift(4), 0);
	assert_eq!(pl.get_leaf_shift(5), 0);
	assert_eq!(pl.get_leaf_shift(6), 4);
	assert_eq!(pl.get_leaf_shift(7), 4);
	assert_eq!(pl.get_leaf_shift(8), 4);

	// now check we can prune some unconnected nodes
	// and that leaf_shift is correct for various pos
	let mut pl = PruneList::empty();
	pl.append(3);
	pl.append(4);
	pl.append(10);
	pl.append(11);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 2);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [6, 13]);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);
	assert_eq!(pl.get_leaf_shift(7), 2);
	assert_eq!(pl.get_leaf_shift(8), 2);
	assert_eq!(pl.get_leaf_shift(12), 4);
	assert_eq!(pl.get_leaf_shift(13), 4);
}

#[test]
fn test_get_shift() {
	let mut pl = PruneList::empty();
	assert!(pl.is_empty());
	assert_eq!(pl.get_shift(0), 0);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);

	// prune a single leaf node
	// pruning only a leaf node does not shift any subsequent pos
	// we will only start shifting when a parent can be pruned
	pl.append(0);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [1]);
	assert_eq!(pl.get_shift(0), 0);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);

	pl.append(1);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [3]);
	assert_eq!(pl.get_shift(0), 0);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 2);
	assert_eq!(pl.get_shift(3), 2);
	assert_eq!(pl.get_shift(4), 2);
	assert_eq!(pl.get_shift(5), 2);

	pl.append(3);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [3, 4]);
	assert_eq!(pl.get_shift(0), 0);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 2);
	assert_eq!(pl.get_shift(3), 2);
	assert_eq!(pl.get_shift(4), 2);
	assert_eq!(pl.get_shift(5), 2);

	pl.append(4);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [7]);
	assert_eq!(pl.get_shift(0), 0);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 0);
	assert_eq!(pl.get_shift(4), 0);
	assert_eq!(pl.get_shift(5), 0);
	assert_eq!(pl.get_shift(6), 6);
	assert_eq!(pl.get_shift(7), 6);
	assert_eq!(pl.get_shift(8), 6);

	// prune a bunch more
	for x in 5..999 {
		if !pl.is_pruned(x) {
			pl.append(x);
		}
	}
	pl.flush().unwrap();

	// and check we shift by a large number (hopefully the correct number...)
	assert_eq!(pl.get_shift(1009), 996);

	// now check we can do some sparse pruning
	let mut pl = PruneList::empty();
	pl.append(3);
	pl.append(4);
	pl.append(7);
	pl.append(8);
	pl.flush().unwrap();

	assert_eq!(pl.iter().collect::<Vec<_>>(), [6, 10]);
	assert_eq!(pl.get_shift(0), 0);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 0);
	assert_eq!(pl.get_shift(4), 0);
	assert_eq!(pl.get_shift(5), 2);
	assert_eq!(pl.get_shift(6), 2);
	assert_eq!(pl.get_shift(7), 2);
	assert_eq!(pl.get_shift(8), 2);
	assert_eq!(pl.get_shift(9), 4);
	assert_eq!(pl.get_shift(10), 4);
	assert_eq!(pl.get_shift(11), 4);
}

#[test]
pub fn test_iter() {
	let mut pl = PruneList::empty();
	pl.append(0);
	pl.append(1);
	pl.append(3);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [3, 4]);

	let mut pl = PruneList::empty();
	pl.append(0);
	pl.append(1);
	pl.append(4);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [3, 5]);
}

#[test]
pub fn test_pruned_bintree_range_iter() {
	let mut pl = PruneList::empty();
	pl.append(0);
	pl.append(1);
	pl.append(3);
	assert_eq!(
		pl.pruned_bintree_range_iter().collect::<Vec<_>>(),
		[1..4, 4..5]
	);

	let mut pl = PruneList::empty();
	pl.append(0);
	pl.append(1);
	pl.append(4);
	assert_eq!(
		pl.pruned_bintree_range_iter().collect::<Vec<_>>(),
		[1..4, 5..6]
	);
}

#[test]
pub fn test_unpruned_iter() {
	let pl = PruneList::empty();
	assert_eq!(pl.unpruned_iter(5).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);

	let mut pl = PruneList::empty();
	pl.append(1);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [2]);
	assert_eq!(pl.pruned_bintree_range_iter().collect::<Vec<_>>(), [2..3]);
	assert_eq!(pl.unpruned_iter(4).collect::<Vec<_>>(), [1, 3, 4]);

	let mut pl = PruneList::empty();
	pl.append(1);
	pl.append(3);
	pl.append(4);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [2, 6]);
	assert_eq!(
		pl.pruned_bintree_range_iter().collect::<Vec<_>>(),
		[2..3, 4..7]
	);
	assert_eq!(pl.unpruned_iter(9).collect::<Vec<_>>(), [1, 3, 7, 8, 9]);
}

#[test]
fn test_unpruned_leaf_iter() {
	let pl = PruneList::empty();
	assert_eq!(
		pl.unpruned_leaf_iter(8).collect::<Vec<_>>(),
		[1, 2, 4, 5, 8]
	);

	let mut pl = PruneList::empty();
	pl.append(1);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [2]);
	assert_eq!(pl.pruned_bintree_range_iter().collect::<Vec<_>>(), [2..3]);
	assert_eq!(pl.unpruned_leaf_iter(5).collect::<Vec<_>>(), [1, 4, 5]);

	let mut pl = PruneList::empty();
	pl.append(1);
	pl.append(3);
	pl.append(4);
	assert_eq!(pl.iter().collect::<Vec<_>>(), [2, 6]);
	assert_eq!(
		pl.pruned_bintree_range_iter().collect::<Vec<_>>(),
		[2..3, 4..7]
	);
	assert_eq!(pl.unpruned_leaf_iter(9).collect::<Vec<_>>(), [1, 8, 9]);
}

pub fn test_append_pruned_subtree() {
	let mut pl = PruneList::empty();

	// append a pruned leaf pos (shift and leaf shift are unaffected).
	pl.append(0);

	assert_eq!(pl.to_vec(), [1]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);

	pl.append(2);

	// subtree beneath root at 2 is pruned
	// pos 3 is shifted by 2 pruned hashes [1, 2]
	// pos 3 is shifted by 2 leaves [1, 2]
	assert_eq!(pl.to_vec(), [3]);
	assert_eq!(pl.get_shift(3), 2);
	assert_eq!(pl.get_leaf_shift(3), 2);

	// append another pruned subtree (ancester of previous one)
	pl.append(6);

	// subtree beneath root at 6 is pruned
	// pos 7 is shifted by 6 pruned hashes [1, 2, 3, 4, 5, 6]
	// pos 3 is shifted by 4 leaves [1, 2, 4, 5]
	assert_eq!(pl.to_vec(), [7]);
	assert_eq!(pl.get_shift(7), 6);
	assert_eq!(pl.get_leaf_shift(7), 4);

	// now append another pruned leaf pos
	pl.append(7);

	// additional pruned leaf does not affect the shift or leaf shift
	// pos 8 is shifted by 6 pruned hashes [1, 2, 3, 4, 5, 6]
	// pos 8 is shifted by 4 leaves [1, 2, 4, 5]
	assert_eq!(pl.to_vec(), [7, 8]);
	assert_eq!(pl.get_shift(8), 6);
	assert_eq!(pl.get_leaf_shift(8), 4);
}

#[test]
fn test_recreate_prune_list() {
	let mut pl = PruneList::empty();
	pl.append(3);
	pl.append(4);
	pl.append(10);

	let pl2 = PruneList::new(None, vec![4, 5, 11].into_iter().collect());

	assert_eq!(pl.to_vec(), pl2.to_vec());
	assert_eq!(pl.shift_cache(), pl2.shift_cache());
	assert_eq!(pl.leaf_shift_cache(), pl2.leaf_shift_cache());

	let pl3 = PruneList::new(None, vec![6, 11].into_iter().collect());

	assert_eq!(pl.to_vec(), pl3.to_vec());
	assert_eq!(pl.shift_cache(), pl3.shift_cache());
	assert_eq!(pl.leaf_shift_cache(), pl3.leaf_shift_cache());
}
