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

use core::pmmr::{bintree_postorder_height, family};

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
#[derive(Default)]
pub struct PruneList {
	/// Vector of pruned nodes positions
	pub pruned_nodes: Vec<u64>,
}

impl PruneList {
	/// Instantiate a new empty prune list
	pub fn new() -> PruneList {
		PruneList {
			pruned_nodes: vec![],
		}
	}

	/// Computes by how many positions a node at pos should be shifted given the
	/// number of nodes that have already been pruned before it. Returns None if
	/// the position has already been pruned.
	pub fn get_shift(&self, pos: u64) -> Option<u64> {
		// get the position where the node at pos would fit in the pruned list, if
		// it's already pruned, nothing to skip

		let pruned_idx = self.next_pruned_idx(pos);
		let next_idx = self.pruned_nodes.binary_search(&pos).map(|x| x + 1).ok();
		match pruned_idx.or(next_idx) {
			None => None,
			Some(idx) => {
				// skip by the number of elements pruned in the preceding subtrees,
				// which is the sum of the size of each subtree
				Some(
					self.pruned_nodes[0..(idx as usize)]
						.iter()
						.map(|n| {
							let height = bintree_postorder_height(*n);
							// height 0, 1 node, offset 0 = 0 + 0
							// height 1, 3 nodes, offset 2 = 1 + 1
							// height 2, 7 nodes, offset 6 = 3 + 3
							// height 3, 15 nodes, offset 14 = 7 + 7
							2 * ((1 << height) - 1)
						})
						.sum(),
				)
			}
		}
	}

	/// As above, but only returning the number of leaf nodes to skip for a
	/// given leaf. Helpful if, for instance, data for each leaf is being stored
	/// separately in a continuous flat-file. Returns None if the position has
	/// already been pruned.
	pub fn get_leaf_shift(&self, pos: u64) -> Option<u64> {
		// get the position where the node at pos would fit in the pruned list, if
		// it's already pruned, nothing to skip

		let pruned_idx = self.next_pruned_idx(pos);
		let next_idx = self.pruned_nodes.binary_search(&pos).map(|x| x + 1).ok();

		let idx = pruned_idx.or(next_idx)?;
		Some(
			// skip by the number of leaf nodes pruned in the preceeding subtrees
			// which just 2^height
			// except in the case of height==0
			// (where we want to treat the pruned tree as 0 leaves)
			self.pruned_nodes[0..(idx as usize)]
				.iter()
				.map(|n| {
					let height = bintree_postorder_height(*n);
					if height == 0 {
						0
					} else {
						(1 << height)
					}
				})
				.sum(),
		)
	}

	/// Push the node at the provided position in the prune list. Compacts the
	/// list if pruning the additional node means a parent can get pruned as
	/// well.
	pub fn add(&mut self, pos: u64) {
		let mut current = pos;
		loop {
			let (parent, sibling) = family(current);

			match self.pruned_nodes.binary_search(&sibling) {
				Ok(idx) => {
					self.pruned_nodes.remove(idx);
					current = parent;
				}
				Err(_) => {
					if let Some(idx) = self.next_pruned_idx(current) {
						self.pruned_nodes.insert(idx, current);
					}
					break;
				}
			}
		}
	}

	/// Checks if the specified position has been pruned,
	/// either directly (pos contained in the prune list itself)
	/// or indirectly (pos is beneath a pruned root).
	pub fn is_pruned(&self, pos: u64) -> bool {
		self.next_pruned_idx(pos).is_none()
	}

	/// Gets the index a new pruned node should take in the prune list.
	/// If the node has already been pruned, either directly or through one of
	/// its parents contained in the prune list, returns None.
	pub fn next_pruned_idx(&self, pos: u64) -> Option<usize> {
		match self.pruned_nodes.binary_search(&pos) {
			Ok(_) => None,
			Err(idx) => {
				if self.pruned_nodes.len() > idx {
					// the node at pos can't be a child of lower position nodes by MMR
					// construction but can be a child of the next node, going up parents
					// from pos to make sure it's not the case
					let next_peak_pos = self.pruned_nodes[idx];
					let mut cursor = pos;
					loop {
						let (parent, _) = family(cursor);
						if next_peak_pos == parent {
							return None;
						}
						if next_peak_pos < parent {
							break;
						}
						cursor = parent;
					}
				}
				Some(idx)
			}
		}
	}
}
