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

//! The Grin "Prune List" implementation.
//!
//! Maintains a set of pruned root node positions that define the pruned
//! and compacted "gaps" in the MMR data and hash files.
//! The root itself is maintained in the hash file, but all positions beneath
//! the root are compacted away. All positions to the right of a pruned node
//! must be shifted the appropriate amount when reading from the hash and data
//! files.

use std::path::{Path, PathBuf};
use std::{
	io::{self, Write},
	ops::Range,
};

use croaring::Bitmap;
use grin_core::core::pmmr;

use crate::core::core::pmmr::{bintree_leftmost, bintree_postorder_height, family};
use crate::{read_bitmap, save_via_temp_file};
use std::cmp::min;

/// Maintains a list of previously pruned nodes in PMMR, compacting the list as
/// parents get pruned and allowing checking whether a leaf is pruned. Given
/// a node's position, computes how much it should get shifted given the
/// subtrees that have been pruned before.
///
/// The PruneList is useful when implementing compact backends for a PMMR (for
/// example a single large byte array or a file). As nodes get pruned and
/// removed from the backend to free space, the backend will get more compact
/// but positions of a node within the PMMR will not match positions in the
/// backend storage anymore. The PruneList accounts for that mismatch and does
/// the position translation.
#[derive(Debug)]
pub struct PruneList {
	path: Option<PathBuf>,
	/// Bitmap representing pruned root node positions.
	bitmap: Bitmap,
	shift_cache: Vec<u64>,
	leaf_shift_cache: Vec<u64>,
}

impl PruneList {
	/// Instantiate a new prune list from the provided path and 1-based bitmap.
	/// Note: Does not flush the bitmap to disk. Caller is responsible for doing this.
	pub fn new(path: Option<PathBuf>, bitmap: Bitmap) -> PruneList {
		assert!(!bitmap.contains(0));
		let mut prune_list = PruneList {
			path,
			bitmap: Bitmap::create(),
			shift_cache: vec![],
			leaf_shift_cache: vec![],
		};

		for pos1 in bitmap.iter() {
			prune_list.append(pos1 as u64 - 1)
		}

		prune_list.bitmap.run_optimize();
		prune_list
	}

	/// Instatiate a new empty prune list.
	pub fn empty() -> PruneList {
		PruneList::new(None, Bitmap::create())
	}

	/// Open an existing prune_list or create a new one.
	/// Takes an optional bitmap of new pruned pos to be combined with existing pos.
	pub fn open<P: AsRef<Path>>(path: P) -> io::Result<PruneList> {
		let file_path = PathBuf::from(path.as_ref());
		let bitmap = if file_path.exists() {
			read_bitmap(&file_path)?
		} else {
			Bitmap::create()
		};
		assert!(!bitmap.contains(0));

		let mut prune_list = PruneList::new(Some(file_path), bitmap);

		// Now build the shift caches from the bitmap we read from disk
		prune_list.init_caches();

		if !prune_list.bitmap.is_empty() {
			debug!(
				"bitmap {} pos ({} bytes), shift_cache {}, leaf_shift_cache {}",
				prune_list.bitmap.cardinality(),
				prune_list.bitmap.get_serialized_size_in_bytes(),
				prune_list.shift_cache.len(),
				prune_list.leaf_shift_cache.len(),
			);
		}

		Ok(prune_list)
	}

	/// Init our internal shift caches.
	pub fn init_caches(&mut self) {
		self.build_shift_cache();
		self.build_leaf_shift_cache();
	}

	/// Save the prune_list to disk.
	pub fn flush(&mut self) -> io::Result<()> {
		// Run the optimization step on the bitmap.
		self.bitmap.run_optimize();

		// Write the updated bitmap file to disk.
		if let Some(ref path) = self.path {
			save_via_temp_file(path, ".tmp", |file| {
				file.write_all(&self.bitmap.serialize())
			})?;
		}

		Ok(())
	}

	/// Return the total shift from all entries in the prune_list.
	/// This is the shift we need to account for when adding new entries to our PMMR.
	pub fn get_total_shift(&self) -> u64 {
		self.get_shift(self.bitmap.maximum().unwrap_or(1) as u64 - 1)
	}

	/// Return the total leaf_shift from all entries in the prune_list.
	/// This is the leaf_shift we need to account for when adding new entries to our PMMR.
	pub fn get_total_leaf_shift(&self) -> u64 {
		self.get_leaf_shift(self.bitmap.maximum().unwrap_or(1) as u64 - 1)
	}

	/// Computes by how many positions a node at pos should be shifted given the
	/// number of nodes that have already been pruned before it.
	/// Note: the node at pos may be pruned and may be compacted away itself and
	/// the caller needs to be aware of this.
	pub fn get_shift(&self, pos0: u64) -> u64 {
		let idx = self.bitmap.rank(1 + pos0 as u32);
		if idx == 0 {
			return 0;
		}
		self.shift_cache[min(idx as usize, self.shift_cache.len()) - 1]
	}

	fn build_shift_cache(&mut self) {
		self.shift_cache.clear();
		for pos1 in self.bitmap.iter() {
			let pos0 = pos1 as u64 - 1;
			let prev_shift = if pos0 == 0 {
				0
			} else {
				self.get_shift(pos0 - 1)
			};

			let curr_shift = if self.is_pruned_root(pos0) {
				let height = bintree_postorder_height(pos0);
				2 * ((1 << height) - 1)
			} else {
				0
			};

			self.shift_cache.push(prev_shift + curr_shift);
		}
	}

	// Calculate the next shift based on provided pos and the previous shift.
	fn calculate_next_shift(&self, pos0: u64) -> u64 {
		let prev_shift = if pos0 == 0 {
			0
		} else {
			self.get_shift(pos0 - 1)
		};
		let shift = if self.is_pruned_root(pos0) {
			let height = bintree_postorder_height(pos0);
			2 * ((1 << height) - 1)
		} else {
			0
		};
		prev_shift + shift
	}

	/// As above, but only returning the number of leaf nodes to skip for a
	/// given leaf. Helpful if, for instance, data for each leaf is being stored
	/// separately in a continuous flat-file.
	pub fn get_leaf_shift(&self, pos0: u64) -> u64 {
		let idx = self.bitmap.rank(1 + pos0 as u32);
		if idx == 0 {
			return 0;
		}
		self.leaf_shift_cache[min(idx as usize, self.leaf_shift_cache.len()) - 1]
	}

	fn build_leaf_shift_cache(&mut self) {
		self.leaf_shift_cache.clear();
		for pos1 in self.bitmap.iter() {
			let pos0 = pos1 as u64 - 1;
			let prev_shift = if pos0 == 0 {
				0
			} else {
				self.get_leaf_shift(pos0 - 1)
			};

			let curr_shift = if self.is_pruned_root(pos0) {
				let height = bintree_postorder_height(pos0);
				if height == 0 {
					0
				} else {
					1 << height
				}
			} else {
				0
			};

			self.leaf_shift_cache.push(prev_shift + curr_shift);
		}
	}

	// Calculate the next leaf shift based on provided pos and the previous leaf shift.
	fn calculate_next_leaf_shift(&self, pos0: u64) -> u64 {
		let prev_shift = if pos0 == 0 {
			0
		} else {
			self.get_leaf_shift(pos0 - 1)
		};
		let shift = if self.is_pruned_root(pos0) {
			let height = bintree_postorder_height(pos0);
			if height == 0 {
				0
			} else {
				1 << height
			}
		} else {
			0
		};
		prev_shift + shift
	}

	// Remove any existing entries in shift_cache and leaf_shift_cache
	// for any pos contained in the subtree with provided root.
	fn cleanup_subtree(&mut self, pos0: u64) {
		let lc0 = bintree_leftmost(pos0) as u32;
		let size = self.bitmap.maximum().unwrap_or(0);

		// If this subtree does not intersect with existing bitmap then nothing to cleanup.
		if lc0 >= size {
			return;
		}

		// Note: We will treat this as a "closed range" below (croaring api weirdness).
		let cleanup_pos1 = (lc0 + 1)..size;

		// Find point where we can truncate based on bitmap "rank" (index) of pos to the left of subtree.
		let idx = self.bitmap.rank(lc0);
		self.shift_cache.truncate(idx as usize);
		self.leaf_shift_cache.truncate(idx as usize);

		self.bitmap.remove_range_closed(cleanup_pos1)
	}

	/// Push the node at the provided position in the prune list.
	/// Assumes rollup of siblings and children has already been handled.
	fn append_single(&mut self, pos0: u64) {
		assert!(
			pos0 >= self.bitmap.maximum().unwrap_or(0) as u64,
			"prune list append only"
		);

		// Add this pos to the bitmap (leaf or subtree root)
		self.bitmap.add(1 + pos0 as u32);

		// Calculate shift and leaf_shift for this pos.
		self.shift_cache.push(self.calculate_next_shift(pos0));
		self.leaf_shift_cache
			.push(self.calculate_next_leaf_shift(pos0));
	}

	/// Push the node at the provided position in the prune list.
	/// Handles rollup of siblings and children as we go (relatively slow).
	/// Once we find a subtree root that can not be rolled up any further
	/// we cleanup everything beneath it and replace it with a single appended node.
	pub fn append(&mut self, pos0: u64) {
		let max = self.bitmap.maximum().unwrap_or(0) as u64;
		assert!(
			pos0 >= max,
			"prune list append only - pos={} bitmap.maximum={}",
			pos0,
			max
		);

		let (parent0, sibling0) = family(pos0);
		if self.is_pruned(sibling0) {
			// Recursively append the parent (removing our sibling in the process).
			self.append(parent0)
		} else {
			// Make sure we roll anything beneath this up into this higher level pruned subtree root.
			// We should have no nested entries in the prune_list.
			self.cleanup_subtree(pos0);
			self.append_single(pos0);
		}
	}

	/// Number of entries in the prune_list.
	pub fn len(&self) -> u64 {
		self.bitmap.cardinality()
	}

	/// Is the prune_list empty?
	pub fn is_empty(&self) -> bool {
		self.bitmap.is_empty()
	}

	/// A pos is pruned if it is a pruned root directly or if it is
	/// beneath the "next" pruned subtree.
	/// We only need to consider the "next" subtree due to the append-only MMR structure.
	pub fn is_pruned(&self, pos0: u64) -> bool {
		if self.is_pruned_root(pos0) {
			return true;
		}
		let rank = self.bitmap.rank(1 + pos0 as u32);
		if let Some(root) = self.bitmap.select(rank as u32) {
			let range = pmmr::bintree_range(root as u64 - 1);
			range.contains(&pos0)
		} else {
			false
		}
	}

	/// Convert the prune_list to a vec of pos.
	pub fn to_vec(&self) -> Vec<u64> {
		self.bitmap.iter().map(|x| x as u64).collect()
	}

	/// Internal shift cache as slice.
	/// only used in store/tests/prune_list.rs tests
	pub fn shift_cache(&self) -> &[u64] {
		self.shift_cache.as_slice()
	}

	/// Internal leaf shift cache as slice.
	/// only used in store/tests/prune_list.rs tests
	pub fn leaf_shift_cache(&self) -> &[u64] {
		self.leaf_shift_cache.as_slice()
	}

	/// Is the specified position a root of a pruned subtree?
	pub fn is_pruned_root(&self, pos0: u64) -> bool {
		self.bitmap.contains(1 + pos0 as u32)
	}

	/// Iterator over the entries in the prune list (pruned roots).
	pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
		self.bitmap.iter().map(|x| x as u64)
	}

	/// Iterator over the pruned "bintree range" for each pruned root.
	pub fn pruned_bintree_range_iter(&self) -> impl Iterator<Item = Range<u64>> + '_ {
		self.iter().map(|x| {
			let rng = pmmr::bintree_range(x - 1);
			(1 + rng.start)..(1 + rng.end)
		})
	}

	/// Iterator over all pos that are *not* pruned based on current prune_list.
	pub fn unpruned_iter(&self, cutoff_pos: u64) -> impl Iterator<Item = u64> + '_ {
		UnprunedIterator::new(self.pruned_bintree_range_iter())
			.take_while(move |x| *x <= cutoff_pos)
	}

	/// Iterator over all leaf pos that are *not* pruned based on current prune_list.
	/// Note this is not necessarily the same as the "leaf_set" as an output
	/// can be spent but not yet pruned.
	pub fn unpruned_leaf_iter(&self, cutoff_pos: u64) -> impl Iterator<Item = u64> + '_ {
		self.unpruned_iter(cutoff_pos)
			.filter(|x| pmmr::is_leaf(*x - 1))
	}

	/// Return a clone of our internal bitmap.
	pub fn bitmap(&self) -> Bitmap {
		self.bitmap.clone()
	}
}

struct UnprunedIterator<I> {
	inner: I,
	current_excl_range: Option<Range<u64>>,
	current_pos: u64,
}

impl<I: Iterator<Item = Range<u64>>> UnprunedIterator<I> {
	fn new(mut inner: I) -> UnprunedIterator<I> {
		let current_excl_range = inner.next();
		UnprunedIterator {
			inner,
			current_excl_range,
			current_pos: 1,
		}
	}
}

impl<I: Iterator<Item = Range<u64>>> Iterator for UnprunedIterator<I> {
	type Item = u64;

	fn next(&mut self) -> Option<Self::Item> {
		if let Some(range) = &self.current_excl_range {
			if self.current_pos < range.start {
				let next = self.current_pos;
				self.current_pos += 1;
				Some(next)
			} else {
				// skip the entire excluded range, moving to next excluded range as necessary
				self.current_pos = range.end;
				self.current_excl_range = self.inner.next();
				self.next()
			}
		} else {
			let next = self.current_pos;
			self.current_pos += 1;
			Some(next)
		}
	}
}
