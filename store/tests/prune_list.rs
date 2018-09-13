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

extern crate grin_store as store;

use store::prune_list::PruneList;

#[test]
fn test_is_pruned() {
	let mut pl = PruneList::new();

	assert_eq!(pl.len(), 0);
	assert_eq!(pl.is_pruned(1), false);
	assert_eq!(pl.is_pruned(2), false);
	assert_eq!(pl.is_pruned(3), false);

	pl.add(2);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), vec![2]);
	assert_eq!(pl.is_pruned(1), false);
	assert_eq!(pl.is_pruned(2), true);
	assert_eq!(pl.is_pruned(3), false);
	assert_eq!(pl.is_pruned(4), false);

	pl.add(2);
	pl.add(1);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 1);
	assert_eq!(pl.to_vec(), [3]);
	assert_eq!(pl.is_pruned(1), true);
	assert_eq!(pl.is_pruned(2), true);
	assert_eq!(pl.is_pruned(3), true);
	assert_eq!(pl.is_pruned(4), false);

	pl.add(4);

	// Flushing the prune_list removes any individual leaf positions.
	// This assumes we will track these outside the prune_list via the leaf_set.
	pl.flush().unwrap();

	assert_eq!(pl.len(), 2);
	assert_eq!(pl.to_vec(), [3, 4]);
	assert_eq!(pl.is_pruned(1), true);
	assert_eq!(pl.is_pruned(2), true);
	assert_eq!(pl.is_pruned(3), true);
	assert_eq!(pl.is_pruned(4), true);
	assert_eq!(pl.is_pruned(5), false);
}

#[test]
fn test_get_leaf_shift() {
	let mut pl = PruneList::new();

	// start with an empty prune list (nothing shifted)
	assert_eq!(pl.len(), 0);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);
	assert_eq!(pl.get_leaf_shift(4), 0);

	// now add a single leaf pos to the prune list
	// leaves will not shift shift anything
	// we only start shifting after pruning a parent
	pl.add(1);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), vec![1]);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);
	assert_eq!(pl.get_leaf_shift(4), 0);

	// now add the sibling leaf pos (pos 1 and pos 2) which will prune the parent
	// at pos 3 this in turn will "leaf shift" the leaf at pos 3 by 2
	pl.add(1);
	pl.add(2);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 1);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 2);
	assert_eq!(pl.get_leaf_shift(4), 2);
	assert_eq!(pl.get_leaf_shift(5), 2);

	// now prune an additional leaf at pos 4
	// leaf offset of subsequent pos will be 2
	// 00100120
	pl.add(4);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 2);
	assert_eq!(pl.to_vec(), [3, 4]);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 2);
	assert_eq!(pl.get_leaf_shift(4), 2);
	assert_eq!(pl.get_leaf_shift(5), 2);
	assert_eq!(pl.get_leaf_shift(6), 2);
	assert_eq!(pl.get_leaf_shift(7), 2);
	assert_eq!(pl.get_leaf_shift(8), 2);

	// now prune the sibling at pos 5
	// the two smaller subtrees (pos 3 and pos 6) are rolled up to larger subtree
	// (pos 7) the leaf offset is now 4 to cover entire subtree containing first
	// 4 leaves 00100120
	pl.add(4);
	pl.add(5);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 1);
	assert_eq!(pl.to_vec(), [7]);
	assert_eq!(pl.get_leaf_shift(1), 0);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(3), 0);
	assert_eq!(pl.get_leaf_shift(4), 0);
	assert_eq!(pl.get_leaf_shift(5), 0);
	assert_eq!(pl.get_leaf_shift(6), 0);
	assert_eq!(pl.get_leaf_shift(7), 4);
	assert_eq!(pl.get_leaf_shift(8), 4);
	assert_eq!(pl.get_leaf_shift(9), 4);

	// now check we can prune some unconnected nodes in arbitrary order
	// and that leaf_shift is correct for various pos
	let mut pl = PruneList::new();
	pl.add(5);
	pl.add(11);
	pl.add(12);
	pl.add(4);
	pl.flush().unwrap();

	assert_eq!(pl.len(), 2);
	assert_eq!(pl.to_vec(), [6, 13]);
	assert_eq!(pl.get_leaf_shift(2), 0);
	assert_eq!(pl.get_leaf_shift(4), 0);
	assert_eq!(pl.get_leaf_shift(8), 2);
	assert_eq!(pl.get_leaf_shift(9), 2);
	assert_eq!(pl.get_leaf_shift(13), 4);
	assert_eq!(pl.get_leaf_shift(14), 4);
}

#[test]
fn test_get_shift() {
	let mut pl = PruneList::new();
	assert!(pl.is_empty());
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 0);

	// prune a single leaf node
	// pruning only a leaf node does not shift any subsequent pos
	// we will only start shifting when a parent can be pruned
	pl.add(1);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), [1]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 0);

	pl.add(1);
	pl.add(2);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), [3]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 2);
	assert_eq!(pl.get_shift(4), 2);
	assert_eq!(pl.get_shift(5), 2);
	assert_eq!(pl.get_shift(6), 2);

	// pos 3 is not a leaf and is already in prune list
	// prune it and check we are still consistent
	pl.add(3);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), [3]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 2);
	assert_eq!(pl.get_shift(4), 2);
	assert_eq!(pl.get_shift(5), 2);
	assert_eq!(pl.get_shift(6), 2);

	pl.add(4);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), [3, 4]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 2);
	assert_eq!(pl.get_shift(4), 2);
	assert_eq!(pl.get_shift(5), 2);
	assert_eq!(pl.get_shift(6), 2);

	pl.add(4);
	pl.add(5);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), [7]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 0);
	assert_eq!(pl.get_shift(4), 0);
	assert_eq!(pl.get_shift(5), 0);
	assert_eq!(pl.get_shift(6), 0);
	assert_eq!(pl.get_shift(7), 6);
	assert_eq!(pl.get_shift(8), 6);
	assert_eq!(pl.get_shift(9), 6);

	// prune a bunch more
	for x in 6..1000 {
		pl.add(x);
	}
	pl.flush().unwrap();

	// and check we shift by a large number (hopefully the correct number...)
	assert_eq!(pl.get_shift(1010), 996);

	let mut pl = PruneList::new();
	pl.add(9);
	pl.add(8);
	pl.add(5);
	pl.add(4);
	pl.flush().unwrap();

	assert_eq!(pl.to_vec(), [6, 10]);
	assert_eq!(pl.get_shift(1), 0);
	assert_eq!(pl.get_shift(2), 0);
	assert_eq!(pl.get_shift(3), 0);
	assert_eq!(pl.get_shift(4), 0);
	assert_eq!(pl.get_shift(5), 0);
	assert_eq!(pl.get_shift(6), 2);
	assert_eq!(pl.get_shift(7), 2);
	assert_eq!(pl.get_shift(8), 2);
	assert_eq!(pl.get_shift(9), 2);
	assert_eq!(pl.get_shift(10), 4);
	assert_eq!(pl.get_shift(11), 4);
	assert_eq!(pl.get_shift(12), 4);
}
