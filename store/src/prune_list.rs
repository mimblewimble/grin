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
//!
//! Maintains a set of pruned root node positions that define the pruned
//! and compacted "gaps" in the MMR data and hash files.
//! The root itself is maintained in the hash file, but all positions beneath
//! the root are compacted away. All positions to the right of a pruned node
//! must be shifted the appropriate amount when reading from the hash and data
//! files.

use std::io::{self, BufWriter, Write};
use std::path::Path;

use croaring::Bitmap;

use crate::core::core::pmmr::{bintree_postorder_height, family, path};
use crate::{read_bitmap, save_via_temp_file};

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
	/// Bitmap representing all pruned node positions (everything under the pruned roots).
	pruned_cache: Bitmap,
	shift_cache: Vec<u64>,
	leaf_shift_cache: Vec<u64>,
}

impl PruneList {
	/// Instantiate a new empty prune list
	pub fn new() -> PruneList {
		PruneList {
			path: None,
			bitmap: Bitmap::create(),
			pruned_cache: Bitmap::create(),
			shift_cache: vec![],
			leaf_shift_cache: vec![],
		}
	}

	/// Open an existing prune_list or create a new one.
	pub fn open(path: &str) -> io::Result<PruneList> {
		let file_path = Path::new(&path);
		let bitmap = if file_path.exists() {
			read_bitmap(&file_path)?
		} else {
			Bitmap::create()
		};

		let mut prune_list = PruneList {
			path: Some(path.to_string()),
			bitmap,
			pruned_cache: Bitmap::create(),
			shift_cache: vec![],
			leaf_shift_cache: vec![],
		};

		// Now built the shift and pruned caches from the bitmap we read from disk.
		prune_list.init_caches();

		if !prune_list.bitmap.is_empty() {
			debug!("prune_list: bitmap {} pos ({} bytes), pruned_cache {} pos ({} bytes), shift_cache {}, leaf_shift_cache {}",
				prune_list.bitmap.cardinality(),
				prune_list.bitmap.get_serialized_size_in_bytes(),
				prune_list.pruned_cache.cardinality(),
				prune_list.pruned_cache.get_serialized_size_in_bytes(),
				prune_list.shift_cache.len(),
				prune_list.leaf_shift_cache.len(),
			);
		}

		Ok(prune_list)
	}

	fn init_caches(&mut self) {
		self.build_shift_cache();
		self.build_leaf_shift_cache();
		self.build_pruned_cache();
	}

	/// Save the prune_list to disk.
	/// Clears out leaf pos before saving to disk
	/// as we track these via the leaf_set.
	pub fn flush(&mut self) -> io::Result<()> {
		// Run the optimization step on the bitmap.
		self.bitmap.run_optimize();

		// Write the updated bitmap file to disk.
		if let Some(ref path) = self.path {
			save_via_temp_file(&path, ".tmp", |w| {
				let mut w = BufWriter::new(w);
				w.write_all(&self.bitmap.serialize())?;
				w.flush()
			})?;
		}

		// Rebuild our "shift caches" here as we are flushing changes to disk
		// and the contents of our prune_list has likely changed.
		self.init_caches();

		Ok(())
	}

	/// Return the total shift from all entries in the prune_list.
	pub fn get_total_shift(&self) -> u64 {
		self.get_shift(self.bitmap.maximum() as u64)
	}

	/// Computes by how many positions a node at pos should be shifted given the
	/// number of nodes that have already been pruned before it.
	/// Note: the node at pos may be pruned and may be compacted away itself and
	/// the caller needs to be aware of this.
	pub fn get_shift(&self, pos: u64) -> u64 {
		if self.bitmap.is_empty() {
			return 0;
		}

		let idx = self.bitmap.rank(pos as u32);
		if idx == 0 {
			return 0;
		}

		if idx > self.shift_cache.len() as u64 {
			self.shift_cache[self.shift_cache.len() - 1]
		} else {
			self.shift_cache[idx as usize - 1]
		}
	}

	fn build_shift_cache(&mut self) {
		if self.bitmap.is_empty() {
			return;
		}

		self.shift_cache.clear();
		for pos in self.bitmap.iter() {
			let pos = pos as u64;
			let prev_shift = self.get_shift(pos - 1);

			let curr_shift = if self.is_pruned_root(pos) {
				let height = bintree_postorder_height(pos);
				2 * ((1 << height) - 1)
			} else {
				0
			};

			self.shift_cache.push(prev_shift + curr_shift);
		}
	}

	/// As above, but only returning the number of leaf nodes to skip for a
	/// given leaf. Helpful if, for instance, data for each leaf is being stored
	/// separately in a continuous flat-file.
	pub fn get_leaf_shift(&self, pos: u64) -> u64 {
		if self.bitmap.is_empty() {
			return 0;
		}

		let idx = self.bitmap.rank(pos as u32);
		if idx == 0 {
			return 0;
		}

		if idx > self.leaf_shift_cache.len() as u64 {
			self.leaf_shift_cache[self.leaf_shift_cache.len() - 1]
		} else {
			self.leaf_shift_cache[idx as usize - 1]
		}
	}

	fn build_leaf_shift_cache(&mut self) {
		if self.bitmap.is_empty() {
			return;
		}

		self.leaf_shift_cache.clear();

		for pos in self.bitmap.iter() {
			let pos = pos as u64;
			let prev_shift = self.get_leaf_shift(pos - 1);

			let curr_shift = if self.is_pruned_root(pos) {
				let height = bintree_postorder_height(pos);
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

	/// Push the node at the provided position in the prune list. Compacts the
	/// list if pruning the additional node means a parent can get pruned as
	/// well.
	pub fn add(&mut self, pos: u64) {
		let mut current = pos;
		loop {
			let (parent, sibling) = family(current);

			if self.bitmap.contains(sibling as u32) || self.pruned_cache.contains(sibling as u32) {
				self.pruned_cache.add(current as u32);
				self.bitmap.remove(sibling as u32);
				current = parent;
			} else {
				self.pruned_cache.add(current as u32);
				self.bitmap.add(current as u32);
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

	/// Is the pos pruned?
	/// Assumes the pruned_cache is fully built and up to date.
	pub fn is_pruned(&self, pos: u64) -> bool {
		self.pruned_cache.contains(pos as u32)
	}

	fn build_pruned_cache(&mut self) {
		if self.bitmap.is_empty() {
			return;
		}
		self.pruned_cache = Bitmap::create_with_capacity(self.bitmap.maximum());
		for pos in 1..(self.bitmap.maximum() + 1) {
			let path = path(pos as u64, self.bitmap.maximum() as u64);
			let pruned = path.into_iter().any(|x| self.bitmap.contains(x as u32));
			if pruned {
				self.pruned_cache.add(pos as u32)
			}
		}
		self.pruned_cache.run_optimize();
	}

	/// Is the specified position a root of a pruned subtree?
	pub fn is_pruned_root(&self, pos: u64) -> bool {
		self.bitmap.contains(pos as u32)
	}
}
