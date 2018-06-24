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

//! The Grin "Prune List" implementation.
//! Currently implemented as a vec of u64 positions.
//! *Soon* to be implemented as a compact bitmap.
//!
//! Maintains a set of pruned root node positions that define the pruned
//! and compacted "gaps" in the MMR data and hash files.
//! The root itself is maintained in the hash file, but all positions beneath
//! the root are compacted away. All positions to the right of a pruned node
//! must be shifted the appropriate amount when reading from the hash and data
//! files.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use croaring::Bitmap;

use core::core::pmmr::{bintree_postorder_height, family, is_leaf, path};

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
pub struct PruneList {
	path: Option<String>,
	/// Bitmap representing pruned root node positions.
	bitmap: Bitmap,
}

impl PruneList {
	/// Instantiate a new empty prune list
	pub fn new() -> PruneList {
		PruneList {
			path: None,
			bitmap: Bitmap::create(),
		}
	}

	/// Open an existing prune_list or create a new one.
	pub fn open(path: String) -> io::Result<PruneList> {
		let file_path = Path::new(&path);
		let bitmap = if file_path.exists() {
			let mut bitmap_file = File::open(path.clone())?;
			let mut buffer = vec![];
			bitmap_file.read_to_end(&mut buffer)?;
			Bitmap::deserialize(&buffer)
		} else {
			Bitmap::create()
		};

		Ok(PruneList {
			path: Some(path.clone()),
			bitmap,
		})
	}

	fn clear_leaves(&mut self) {
		let mut leaf_pos = Bitmap::create();
		for x in self.bitmap.iter() {
			if is_leaf(x as u64) {
				leaf_pos.add(x);
			}
		}
		self.bitmap.andnot_inplace(&leaf_pos);
	}

	/// Save the prune_list to disk.
	/// Clears out leaf pos before saving to disk
	/// as we track these via the leaf_set.
	pub fn flush(&mut self) -> io::Result<()> {
		// First clear any leaf pos from the prune_list (these are tracked via the
		// leaf_set).
		self.clear_leaves();

		// Now run the optimization step on the bitmap.
		self.bitmap.run_optimize();

		// TODO - consider writing this to disk in a tmp file and then renaming?

		// Write the updated bitmap file to disk.
		if let Some(ref path) = self.path {
			let mut file = BufWriter::new(File::create(path)?);
			file.write_all(&self.bitmap.serialize())?;
			file.flush()?;
		}

		Ok(())
	}

	/// Return the total shift from all entries in the prune_list.
	pub fn get_total_shift(&self) -> u64 {
		self.get_shift(self.bitmap.maximum() as u64 + 1)
	}

	/// Computes by how many positions a node at pos should be shifted given the
	/// number of nodes that have already been pruned before it.
	/// Note: the node at pos may be pruned and may be compacted away itself and
	/// the caller needs to be aware of this.
	pub fn get_shift(&self, pos: u64) -> u64 {
		let pruned = self.pruned_lte(pos);

		// skip by the number of leaf nodes pruned in the preceeding subtrees
		// which just 2^height
		// except in the case of height==0
		// (where we want to treat the pruned tree as 0 leaves)
		pruned
			.iter()
			.map(|n| {
				let height = bintree_postorder_height(*n);
				// height 0, 1 node, offset 0 = 0 + 0
				// height 1, 3 nodes, offset 2 = 1 + 1
				// height 2, 7 nodes, offset 6 = 3 + 3
				// height 3, 15 nodes, offset 14 = 7 + 7
				2 * ((1 << height) - 1)
			})
			.sum()
	}

	/// As above, but only returning the number of leaf nodes to skip for a
	/// given leaf. Helpful if, for instance, data for each leaf is being stored
	/// separately in a continuous flat-file.
	pub fn get_leaf_shift(&self, pos: u64) -> u64 {
		let pruned = self.pruned_lte(pos);

		// skip by the number of leaf nodes pruned in the preceeding subtrees
		// which just 2^height
		// except in the case of height==0
		// (where we want to treat the pruned tree as 0 leaves)
		pruned
			.iter()
			.map(|&n| {
				let height = bintree_postorder_height(n);
				if height == 0 {
					0
				} else {
					1 << height
				}
			})
			.sum()
	}

	/// Push the node at the provided position in the prune list. Compacts the
	/// list if pruning the additional node means a parent can get pruned as
	/// well.
	pub fn add(&mut self, pos: u64) {
		let mut current = pos;
		loop {
			let (parent, sibling) = family(current);

			if self.bitmap.contains(sibling as u32) {
				self.bitmap.remove(sibling as u32);
				current = parent;
			} else {
				if !self.is_pruned(current) {
					self.bitmap.add(current as u32);
				}
				break;
			}
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

	/// Convert the prune_list to a vec of pos.
	pub fn to_vec(&self) -> Vec<u64> {
		self.bitmap.to_vec().into_iter().map(|x| x as u64).collect()
	}

	/// Checks if the specified position has been pruned,
	/// either directly (pos contained in the prune list itself)
	/// or indirectly (pos is beneath a pruned root).
	pub fn is_pruned(&self, pos: u64) -> bool {
		if self.is_empty() {
			return false;
		}

		let path = path(pos, self.bitmap.maximum() as u64);
		path.into_iter().any(|x| self.bitmap.contains(x as u32))
	}

	/// Is the specified position a root of a pruned subtree?
	pub fn is_pruned_root(&self, pos: u64) -> bool {
		self.bitmap.contains(pos as u32)
	}

	fn pruned_lte(&self, pos: u64) -> Vec<u64> {
		let mut res = vec![];
		for x in self.bitmap.iter() {
			if x > pos as u32 {
				break;
			}
			res.push(x as u64);
		}
		res
	}
}
