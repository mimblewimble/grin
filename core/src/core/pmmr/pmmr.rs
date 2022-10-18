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

use std::{iter, marker, ops::Range, u64};

use croaring::Bitmap;

use crate::core::hash::{Hash, ZERO_HASH};
use crate::core::merkle_proof::MerkleProof;
use crate::core::pmmr::{Backend, ReadonlyPMMR};
use crate::core::BlockHeader;
use crate::ser::{PMMRIndexHashable, PMMRable};

/// Trait with common methods for reading from a PMMR
pub trait ReadablePMMR {
	/// Leaf type
	type Item;

	/// Get the hash at provided position in the MMR.
	/// NOTE all positions are 0-based, so a size n MMR has nodes in positions 0 through n-1
	/// just like a Rust Range 0..n
	fn get_hash(&self, pos: u64) -> Option<Hash>;

	/// Get the data element at provided position in the MMR.
	fn get_data(&self, pos: u64) -> Option<Self::Item>;

	/// Get the hash from the underlying MMR file (ignores the remove log).
	fn get_from_file(&self, pos: u64) -> Option<Hash>;

	/// Get the hash for the provided peak pos.
	/// Optimized for reading peak hashes rather than arbitrary pos hashes.
	/// Peaks can be assumed to not be compacted.
	fn get_peak_from_file(&self, pos: u64) -> Option<Hash>;

	/// Get the data element at provided position in the MMR (ignores the remove log).
	fn get_data_from_file(&self, pos: u64) -> Option<Self::Item>;

	/// Total size of the tree, including intermediary nodes and ignoring any pruning.
	fn unpruned_size(&self) -> u64;

	/// Iterator over current (unpruned, unremoved) leaf positions.
	fn leaf_pos_iter(&self) -> Box<dyn Iterator<Item = u64> + '_>;

	/// Iterator over current (unpruned, unremoved) leaf insertion indices.
	fn leaf_idx_iter(&self, from_idx: u64) -> Box<dyn Iterator<Item = u64> + '_>;

	/// Number of leaves in the MMR
	fn n_unpruned_leaves(&self) -> u64;

	/// Number of leaves in the MMR up to index
	fn n_unpruned_leaves_to_index(&self, to_index: u64) -> u64;

	/// Is the MMR empty?
	fn is_empty(&self) -> bool {
		self.unpruned_size() == 0
	}

	/// Takes a single peak position and hashes together
	/// all the peaks to the right of this peak (if any).
	/// If this return a hash then this is our peaks sibling.
	/// If none then the sibling of our peak is the peak to the left.
	fn bag_the_rhs(&self, peak_pos0: u64) -> Option<Hash> {
		let size = self.unpruned_size();
		let rhs = peaks(size)
			.into_iter()
			.filter(|&x| x > peak_pos0)
			.filter_map(|x| self.get_from_file(x));

		let mut res = None;
		for peak in rhs.rev() {
			res = match res {
				None => Some(peak),
				Some(rhash) => Some((peak, rhash).hash_with_index(size)),
			}
		}
		res
	}

	/// Returns a vec of the peaks of this MMR.
	fn peaks(&self) -> Vec<Hash> {
		peaks(self.unpruned_size())
			.into_iter()
			.filter_map(move |pi0| self.get_peak_from_file(pi0))
			.collect()
	}

	/// Hashes of the peaks excluding `peak_pos`, where the rhs is bagged together
	fn peak_path(&self, peak_pos0: u64) -> Vec<Hash> {
		let rhs = self.bag_the_rhs(peak_pos0);
		let mut res = peaks(self.unpruned_size())
			.into_iter()
			.filter(|&x| x < peak_pos0)
			.filter_map(|x| self.get_peak_from_file(x))
			.collect::<Vec<_>>();
		if let Some(rhs) = rhs {
			res.push(rhs);
		}
		res.reverse();

		res
	}

	/// Computes the root of the MMR. Find all the peaks in the current
	/// tree and "bags" them to get a single peak.
	fn root(&self) -> Result<Hash, String> {
		if self.is_empty() {
			return Ok(ZERO_HASH);
		}
		let mut res = None;
		let peaks = self.peaks();
		let mmr_size = self.unpruned_size();
		for peak in peaks.into_iter().rev() {
			res = match res {
				None => Some(peak),
				Some(rhash) => Some((peak, rhash).hash_with_index(mmr_size)),
			}
		}
		res.ok_or_else(|| "no root, invalid tree".to_owned())
	}

	/// Build a Merkle proof for the element at the given position.
	fn merkle_proof(&self, pos0: u64) -> Result<MerkleProof, String> {
		let size = self.unpruned_size();
		debug!("merkle_proof  {}, size {}", pos0, size);

		// check this pos is actually a leaf in the MMR
		if !is_leaf(pos0) {
			return Err(format!("not a leaf at pos {}", pos0));
		}

		// check we actually have a hash in the MMR at this pos
		self.get_hash(pos0)
			.ok_or_else(|| format!("no element at pos {}", pos0))?;

		let family_branch = family_branch(pos0, size);

		let mut path = family_branch
			.iter()
			.filter_map(|x| self.get_from_file(x.1))
			.collect::<Vec<_>>();

		let peak_pos = match family_branch.last() {
			Some(&(x, _)) => x,
			None => pos0,
		};

		path.append(&mut self.peak_path(peak_pos));

		Ok(MerkleProof {
			mmr_size: size,
			path,
		})
	}
}

/// Prunable Merkle Mountain Range implementation. All positions within the tree
/// start at 0 just like array indices.
///
/// Heavily relies on navigation operations within a binary tree. In particular,
/// all the implementation needs to keep track of the MMR structure is how far
/// we are in the sequence of nodes making up the MMR.
pub struct PMMR<'a, T, B>
where
	T: PMMRable,
	B: Backend<T>,
{
	/// Number of nodes in the PMMR
	pub size: u64,
	backend: &'a mut B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> PMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	/// Build a new prunable Merkle Mountain Range using the provided backend.
	pub fn new(backend: &'a mut B) -> PMMR<'_, T, B> {
		PMMR {
			backend,
			size: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new prunable Merkle Mountain Range pre-initialized until
	/// size with the provided backend.
	pub fn at(backend: &'a mut B, size: u64) -> PMMR<'_, T, B> {
		PMMR {
			backend,
			size,
			_marker: marker::PhantomData,
		}
	}

	/// Build a "readonly" view of this PMMR.
	pub fn readonly_pmmr(&self) -> ReadonlyPMMR<'_, T, B> {
		ReadonlyPMMR::at(&self.backend, self.size)
	}

	/// Push a new element into the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push(&mut self, leaf: &T) -> Result<u64, String> {
		let leaf_pos = self.size;
		let mut current_hash = leaf.hash_with_index(leaf_pos);

		let mut hashes = vec![current_hash];
		let mut pos = leaf_pos;

		let (peak_map, height) = peak_map_height(pos);
		if height != 0 {
			return Err(format!("bad mmr size {}", pos));
		}
		// hash with all immediately preceding peaks, as indicated by peak map
		let mut peak = 1;
		while (peak_map & peak) != 0 {
			let left_sibling = pos + 1 - 2 * peak;
			let left_hash = self
				.backend
				.get_peak_from_file(left_sibling)
				.ok_or("missing left sibling in tree, should not have been pruned")?;
			peak *= 2;
			pos += 1;
			current_hash = (left_hash, current_hash).hash_with_index(pos);
			hashes.push(current_hash);
		}

		// append all the new nodes and update the MMR index
		self.backend.append(leaf, &hashes)?;
		self.size = pos + 1;
		Ok(leaf_pos)
	}

	/// Push a pruned subtree into the PMMR
	pub fn push_pruned_subtree(&mut self, hash: Hash, pos0: u64) -> Result<(), String> {
		// First append the subtree
		self.backend.append_pruned_subtree(hash, pos0)?;
		self.size = pos0 + 1;

		let mut pos = pos0;
		let mut current_hash = hash;

		let (peak_map, _) = peak_map_height(pos);

		// Then hash with all immediately preceding peaks, as indicated by peak map
		let mut peak = 1;
		while (peak_map & peak) != 0 {
			let (parent, sibling) = family(pos);
			peak *= 2;
			if sibling > pos {
				// is right sibling, we should be done
				continue;
			}
			let left_hash = self
				.backend
				.get_hash(sibling)
				.ok_or("missing left sibling in tree, should not have been pruned")?;
			pos = parent;
			current_hash = (left_hash, current_hash).hash_with_index(parent);
			self.backend.append_hash(current_hash)?;
		}

		// Round size up to next leaf, ready for insertion
		self.size = crate::core::pmmr::round_up_to_leaf_pos(pos);
		Ok(())
	}

	/// Reset prune list
	pub fn reset_prune_list(&mut self) {
		self.backend.reset_prune_list();
	}

	/// Remove the specified position from the leaf set
	pub fn remove_from_leaf_set(&mut self, pos0: u64) {
		self.backend.remove_from_leaf_set(pos0);
	}

	/// Saves a snapshot of the MMR tagged with the block hash.
	/// Specifically - snapshots the utxo file as we need this rewound before
	/// sending the txhashset zip file to another node for fast-sync.
	pub fn snapshot(&mut self, header: &BlockHeader) -> Result<(), String> {
		self.backend.snapshot(header)?;
		Ok(())
	}

	/// Rewind the PMMR to a previous position, as if all push operations after
	/// that had been canceled. Expects a position in the PMMR to rewind and
	/// bitmaps representing the positions added and removed that we want to
	/// "undo".
	pub fn rewind(&mut self, position: u64, rewind_rm_pos: &Bitmap) -> Result<(), String> {
		// Identify which actual position we should rewind to as the provided
		// position is a leaf. We traverse the MMR to include any parent(s) that
		// need to be included for the MMR to be valid.
		let leaf_pos = round_up_to_leaf_pos(position);
		self.backend.rewind(leaf_pos, rewind_rm_pos)?;
		self.size = leaf_pos;
		Ok(())
	}

	/// Prunes (removes) the leaf from the MMR at the specified position.
	/// Returns an error if prune is called on a non-leaf position.
	/// Returns false if the leaf node has already been pruned.
	/// Returns true if pruning is successful.
	pub fn prune(&mut self, pos0: u64) -> Result<bool, String> {
		if !is_leaf(pos0) {
			return Err(format!("Node at {} is not a leaf, can't prune.", pos0));
		}

		if self.backend.get_hash(pos0).is_none() {
			return Ok(false);
		}

		self.backend.remove(pos0)?;
		Ok(true)
	}

	/// Walks all unpruned nodes in the MMR and revalidate all parent hashes
	pub fn validate(&self) -> Result<(), String> {
		// iterate on all parent nodes
		for n in 0..self.size {
			let height = bintree_postorder_height(n);
			if height > 0 {
				if let Some(hash) = self.get_hash(n) {
					let left_pos = n - (1 << height);
					let right_pos = n - 1;
					// using get_from_file here for the children (they may have been "removed")
					if let Some(left_child_hs) = self.get_from_file(left_pos) {
						if let Some(right_child_hs) = self.get_from_file(right_pos) {
							// hash the two child nodes together with parent_pos and compare
							if (left_child_hs, right_child_hs).hash_with_index(n) != hash {
								return Err(format!(
									"Invalid MMR, hash of parent at {} does \
									 not match children.",
									n + 1
								));
							}
						}
					}
				}
			}
		}
		Ok(())
	}

	/// Debugging utility to print information about the MMRs. Short version
	/// only prints the last 8 nodes.
	pub fn dump(&self, short: bool) {
		let sz = self.unpruned_size();
		if sz > 2000 && !short {
			return;
		}
		let start = if short { sz / 8 } else { 0 };
		for n in start..(sz / 8 + 1) {
			let mut idx = "".to_owned();
			let mut hashes = "".to_owned();
			for m in (n * 8)..(n + 1) * 8 {
				if m >= sz {
					break;
				}
				idx.push_str(&format!("{:>8} ", m));
				let ohs = self.get_hash(m);
				match ohs {
					Some(hs) => hashes.push_str(&format!("{} ", hs)),
					None => hashes.push_str(&format!("{:>8} ", "??")),
				}
			}
			debug!("{}", idx);
			debug!("{}", hashes);
		}
	}

	/// Prints PMMR statistics to the logs, used for debugging.
	pub fn dump_stats(&self) {
		debug!("pmmr: unpruned - {}", self.unpruned_size());
		self.backend.dump_stats();
	}

	/// Debugging utility to print information about the MMRs. Short version
	/// only prints the last 8 nodes.
	/// Looks in the underlying hash file and so ignores the remove log.
	pub fn dump_from_file(&self, short: bool) {
		let sz = self.unpruned_size();
		if sz > 2000 && !short {
			return;
		}
		let start = if short { sz / 8 } else { 0 };
		for n in start..(sz / 8 + 1) {
			let mut idx = "".to_owned();
			let mut hashes = "".to_owned();
			for m in (n * 8)..(n + 1) * 8 {
				if m >= sz {
					break;
				}
				idx.push_str(&format!("{:>8} ", m + 1));
				let ohs = self.get_from_file(m);
				match ohs {
					Some(hs) => hashes.push_str(&format!("{} ", hs)),
					None => hashes.push_str(&format!("{:>8} ", " .")),
				}
			}
			debug!("{}", idx);
			debug!("{}", hashes);
		}
	}
}

impl<'a, T, B> ReadablePMMR for PMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	type Item = T::E;

	fn get_hash(&self, pos0: u64) -> Option<Hash> {
		if pos0 >= self.size {
			None
		} else if is_leaf(pos0) {
			// If we are a leaf then get hash from the backend.
			self.backend.get_hash(pos0)
		} else {
			// If we are not a leaf get hash ignoring the remove log.
			self.backend.get_from_file(pos0)
		}
	}

	fn get_data(&self, pos0: u64) -> Option<Self::Item> {
		if pos0 >= self.size {
			// If we are beyond the rhs of the MMR return None.
			None
		} else if is_leaf(pos0) {
			// If we are a leaf then get data from the backend.
			self.backend.get_data(pos0)
		} else {
			// If we are not a leaf then return None as only leaves have data.
			None
		}
	}

	fn get_from_file(&self, pos0: u64) -> Option<Hash> {
		if pos0 >= self.size {
			None
		} else {
			self.backend.get_from_file(pos0)
		}
	}

	fn get_peak_from_file(&self, pos0: u64) -> Option<Hash> {
		if pos0 >= self.size {
			None
		} else {
			self.backend.get_peak_from_file(pos0)
		}
	}

	fn get_data_from_file(&self, pos0: u64) -> Option<Self::Item> {
		if pos0 >= self.size {
			None
		} else {
			self.backend.get_data_from_file(pos0)
		}
	}

	fn unpruned_size(&self) -> u64 {
		self.size
	}

	fn leaf_pos_iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
		self.backend.leaf_pos_iter()
	}

	fn leaf_idx_iter(&self, from_idx: u64) -> Box<dyn Iterator<Item = u64> + '_> {
		self.backend.leaf_idx_iter(from_idx)
	}

	fn n_unpruned_leaves(&self) -> u64 {
		self.backend.n_unpruned_leaves()
	}

	fn n_unpruned_leaves_to_index(&self, to_index: u64) -> u64 {
		self.backend.n_unpruned_leaves_to_index(to_index)
	}
}

/// 64 bits all ones: 0b11111111...1
const ALL_ONES: u64 = u64::MAX;

/// peak bitmap and height of next node in mmr of given size
/// Example: on size 4 returns (0b11, 0) as mmr tree of size 4 is
///    2
///   / \
///  0   1   3
/// with 0b11 indicating the presence of peaks of height 0 and 1,
/// and 0 the height of the next node 4, which is a leaf
/// NOTE:
/// the peak map also encodes the path taken from the root to the added node
/// since the path turns left (resp. right) if-and-only-if
/// a peak at that height is absent (resp. present)
pub fn peak_map_height(mut size: u64) -> (u64, u64) {
	if size == 0 {
		// rust can't shift right by 64
		return (0, 0);
	}
	let mut peak_size = ALL_ONES >> size.leading_zeros();
	let mut peak_map = 0;
	while peak_size != 0 {
		peak_map <<= 1;
		if size >= peak_size {
			size -= peak_size;
			peak_map |= 1;
		}
		peak_size >>= 1;
	}
	(peak_map, size)
}

/// sizes of peaks and height of next node in mmr of given size
/// similar to peak_map_height but replacing bitmap by vector of sizes
/// Example: on input 5 returns ([3,1], 1) as mmr state before adding 5 was
///    2
///   / \
///  0   1   3   4
pub fn peak_sizes_height(mut size: u64) -> (Vec<u64>, u64) {
	if size == 0 {
		// rust can't shift right by 64
		return (vec![], 0);
	}
	let mut peak_size = ALL_ONES >> size.leading_zeros();
	let mut peak_sizes = vec![];
	while peak_size != 0 {
		if size >= peak_size {
			peak_sizes.push(peak_size);
			size -= peak_size;
		}
		peak_size >>= 1;
	}
	(peak_sizes, size)
}

/// Gets the postorder traversal 0-based index of all peaks in a MMR given its size.
/// Starts with the top peak, which is always on the left
/// side of the range, and navigates toward lower siblings toward the right
/// of the range.
/// For some odd reason, return empty when next node is not a leaf
pub fn peaks(size: u64) -> Vec<u64> {
	let (peak_sizes, height) = peak_sizes_height(size);
	if height == 0 {
		peak_sizes
			.iter()
			.scan(0, |acc, &x| {
				*acc += &x;
				Some(*acc)
			})
			.map(|x| x - 1) // rust doesn't allow starting scan with -1 as u64
			.collect()
	} else {
		vec![]
	}
}
/// The number of leaves in a MMR of the provided size.
pub fn n_leaves(size: u64) -> u64 {
	let (peak_map, height) = peak_map_height(size);
	if height == 0 {
		peak_map
	} else {
		peak_map + 1
	}
}

/// returns least position >= pos0 with height 0
pub fn round_up_to_leaf_pos(pos0: u64) -> u64 {
	let (insert_idx, height) = peak_map_height(pos0);
	let leaf_idx = if height == 0 {
		insert_idx
	} else {
		insert_idx + 1
	};
	return insertion_to_pmmr_index(leaf_idx);
}

/// Returns the 0-based pmmr index of 0-based leaf index n
pub fn insertion_to_pmmr_index(nleaf0: u64) -> u64 {
	2 * nleaf0 - nleaf0.count_ones() as u64
}

/// Returns the insertion index of the given leaf index
pub fn pmmr_leaf_to_insertion_index(pos0: u64) -> Option<u64> {
	let (insert_idx, height) = peak_map_height(pos0);
	if height == 0 {
		Some(insert_idx)
	} else {
		None
	}
}

/// The height of a node in a full binary tree from its postorder traversal
/// index.
pub fn bintree_postorder_height(pos0: u64) -> u64 {
	peak_map_height(pos0).1
}

/// Is this position a leaf in the MMR?
/// We know the positions of all leaves based on the postorder height of an MMR
/// of any size (somewhat unintuitively but this is how the PMMR is "append
/// only").
pub fn is_leaf(pos0: u64) -> bool {
	bintree_postorder_height(pos0) == 0
}

/// Calculates the positions of the parent and sibling of the node at the
/// provided position.
pub fn family(pos0: u64) -> (u64, u64) {
	let (peak_map, height) = peak_map_height(pos0);
	let peak = 1 << height;
	if (peak_map & peak) != 0 {
		(pos0 + 1, pos0 + 1 - 2 * peak)
	} else {
		(pos0 + 2 * peak, pos0 + 2 * peak - 1)
	}
}

/// Is the node at this pos the "left" sibling of its parent?
pub fn is_left_sibling(pos0: u64) -> bool {
	let (peak_map, height) = peak_map_height(pos0);
	let peak = 1 << height;
	(peak_map & peak) == 0
}

/// For a given starting position calculate the parent and sibling positions
/// for the branch/path from that position to the peak of the tree.
/// We will use the sibling positions to generate the "path" of a Merkle proof.
pub fn family_branch(pos0: u64, size: u64) -> Vec<(u64, u64)> {
	// loop going up the tree, from node to parent, as long as we stay inside
	// the tree (as defined by size).
	let (peak_map, height) = peak_map_height(pos0);
	let mut peak = 1 << height;
	let mut branch = vec![];
	let mut current = pos0;
	let mut sibling;
	while current + 1 < size {
		if (peak_map & peak) != 0 {
			current += 1;
			sibling = current - 2 * peak;
		} else {
			current += 2 * peak;
			sibling = current - 1;
		};
		if current >= size {
			break;
		}
		branch.push((current, sibling));
		peak <<= 1;
	}
	branch
}

/// Gets the position of the rightmost node (i.e. leaf) beneath the provided subtree root.
pub fn bintree_rightmost(pos0: u64) -> u64 {
	pos0 - bintree_postorder_height(pos0)
}

/// Gets the position of the leftmost node (i.e. leaf) beneath the provided subtree root.
pub fn bintree_leftmost(pos0: u64) -> u64 {
	let height = bintree_postorder_height(pos0);
	pos0 + 2 - (2 << height)
}

/// Iterator over all leaf pos beneath the provided subtree root (including the root itself).
pub fn bintree_leaf_pos_iter(pos0: u64) -> Box<dyn Iterator<Item = u64>> {
	let leaf_start = pmmr_leaf_to_insertion_index(bintree_leftmost(pos0));
	let leaf_end = pmmr_leaf_to_insertion_index(bintree_rightmost(pos0));
	let leaf_start = match leaf_start {
		Some(l) => l,
		None => return Box::new(iter::empty::<u64>()),
	};
	let leaf_end = match leaf_end {
		Some(l) => l,
		None => return Box::new(iter::empty::<u64>()),
	};
	Box::new((leaf_start..=leaf_end).map(|n| insertion_to_pmmr_index(n)))
}

/// Iterator over all pos beneath the provided subtree root (including the root itself).
pub fn bintree_pos_iter(pos0: u64) -> impl Iterator<Item = u64> {
	let leaf_start = bintree_leftmost(pos0);
	(leaf_start..=pos0).into_iter()
}

/// All pos in the subtree beneath the provided root, including root itself.
pub fn bintree_range(pos0: u64) -> Range<u64> {
	let height = bintree_postorder_height(pos0);
	let leftmost = pos0 + 2 - (2 << height);
	leftmost..(pos0 + 1)
}
