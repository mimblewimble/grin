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

//! Persistent and prunable Merkle Mountain Range implementation. For a high
//! level description of MMRs, see:
//!
//! https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md
//!
//! This implementation is built in two major parts:
//!
//! 1. A set of low-level functions that allow navigation within an arbitrary
//! sized binary tree traversed in postorder. To realize why this us useful,
//! we start with the standard height sequence in a MMR: 0010012001... This is
//! in fact identical to the postorder traversal (left-right-top) of a binary
//! tree. In addition postorder traversal is independent of the height of the
//! tree. This allows us, with a few primitive, to get the height of any node
//! in the MMR from its position in the sequence, as well as calculate the
//! position of siblings, parents, etc. As all those functions only rely on
//! binary operations, they're extremely fast. For more information, see the
//! doc on bintree_jump_left_sibling.
//! 2. The implementation of a prunable MMR tree using the above. Each leaf
//! is required to be Writeable (which implements Hashed). Tree roots can be
//! trivially and efficiently calculated without materializing the full tree.
//! The underlying Hashes are stored in a Backend implementation that can
//! either be a simple Vec or a database.

use std::clone::Clone;
use std::marker;
use core::hash::Hash;
use core::MerkleProof;
use ser::{PMMRIndexHashable, PMMRable};
use util::LOGGER;

/// Storage backend for the MMR, just needs to be indexed by order of insertion.
/// The PMMR itself does not need the Backend to be accurate on the existence
/// of an element (i.e. remove could be a no-op) but layers above can
/// depend on an accurate Backend to check existence.
pub trait Backend<T>
where
	T: PMMRable,
{
	/// Append the provided Hashes to the backend storage, and optionally an associated
	/// data element to flatfile storage (for leaf nodes only). The position of the
	/// first element of the Vec in the MMR is provided to help the implementation.
	fn append(&mut self, position: u64, data: Vec<(Hash, Option<T>)>) -> Result<(), String>;

	/// Rewind the backend state to a previous position, as if all append
	/// operations after that had been canceled. Expects a position in the PMMR
	/// to rewind to as well as the consumer-provided index of when the change
	/// occurred (see remove).
	fn rewind(&mut self, position: u64, index: u32) -> Result<(), String>;

	/// Get a Hash by insertion position.
	fn get_hash(&self, position: u64) -> Option<Hash>;

	/// Get underlying data by insertion position.
	fn get_data(&self, position: u64) -> Option<T>;

	/// Get a Hash  by original insertion position
	/// (ignoring the remove log).
	fn get_from_file(&self, position: u64) -> Option<Hash>;

	/// Get a Data Element by original insertion position
	/// (ignoring the remove log).
	fn get_data_from_file(&self, position: u64) -> Option<T>;

	/// Remove HashSums by insertion position. An index is also provided so the
	/// underlying backend can implement some rollback of positions up to a
	/// given index (practically the index is the height of a block that
	/// triggered removal).
	fn remove(&mut self, positions: Vec<u64>, index: u32) -> Result<(), String>;

	/// Returns the data file path.. this is a bit of a hack now that doesn't
	/// sit well with the design, but TxKernels have to be summed and the
	/// fastest way to to be able to allow direct access to the file
	fn get_data_file_path(&self) -> String;

	/// For debugging purposes so we can see how compaction is doing.
	fn dump_stats(&self);
}

/// Maximum peaks for a Merkle proof
pub const MAX_PEAKS: u64 = 100;

/// Maximum path for a Merkle proof
pub const MAX_PATH: u64 = 100;

/// Prunable Merkle Mountain Range implementation. All positions within the tree
/// start at 1 as they're postorder tree traversal positions rather than array
/// indices.
///
/// Heavily relies on navigation operations within a binary tree. In particular,
/// all the implementation needs to keep track of the MMR structure is how far
/// we are in the sequence of nodes making up the MMR.
pub struct PMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	last_pos: u64,
	backend: &'a mut B,
	// only needed for parameterizing Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> PMMR<'a, T, B>
where
	T: PMMRable + ::std::fmt::Debug,
	B: 'a + Backend<T>,
{
	/// Build a new prunable Merkle Mountain Range using the provided backend.
	pub fn new(backend: &'a mut B) -> PMMR<T, B> {
		PMMR {
			last_pos: 0,
			backend: backend,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new prunable Merkle Mountain Range pre-initialized until
	/// last_pos
	/// with the provided backend.
	pub fn at(backend: &'a mut B, last_pos: u64) -> PMMR<T, B> {
		PMMR {
			last_pos: last_pos,
			backend: backend,
			_marker: marker::PhantomData,
		}
	}

	/// Returns a vec of the peaks of this MMR.
	pub fn peaks(&self) -> Vec<Hash> {
		let peaks_pos = peaks(self.last_pos);
		peaks_pos
			.into_iter()
			.filter_map(|pi| {
				// here we want to get from underlying hash file
				// as the pos *may* have been "removed"
				self.backend.get_from_file(pi)
			})
			.collect()
	}

	fn peak_path(&self, peak_pos: u64) -> Vec<Hash> {
		let rhs = self.bag_the_rhs(peak_pos);
		let mut res = peaks(self.last_pos)
			.into_iter()
			.filter(|x| x < &peak_pos)
			.filter_map(|x| self.backend.get_from_file(x))
			.collect::<Vec<_>>();
		res.reverse();
		if let Some(rhs) = rhs {
			res.insert(0, rhs);
		}
		res
	}

	/// Takes a single peak position and hashes together
	/// all the peaks to the right of this peak (if any).
	/// If this return a hash then this is our peaks sibling.
	/// If none then the sibling of our peak is the peak to the left.
	pub fn bag_the_rhs(&self, peak_pos: u64) -> Option<Hash> {
		let rhs = peaks(self.last_pos)
			.into_iter()
			.filter(|x| x > &peak_pos)
			.filter_map(|x| self.backend.get_from_file(x))
			.collect::<Vec<_>>();

		let mut res = None;
		for peak in rhs.iter().rev() {
			res = match res {
				None => Some(*peak),
				Some(rhash) => Some((*peak, rhash).hash_with_index(self.unpruned_size())),
			}
		}
		res
	}

	/// Computes the root of the MMR. Find all the peaks in the current
	/// tree and "bags" them to get a single peak.
	pub fn root(&self) -> Hash {
		let mut res = None;
		for peak in self.peaks().iter().rev() {
			res = match res {
				None => Some(*peak),
				Some(rhash) => Some((*peak, rhash).hash_with_index(self.unpruned_size())),
			}
		}
		res.expect("no root, invalid tree")
	}

	/// Build a Merkle proof for the element at the given position.
	pub fn merkle_proof(&self, pos: u64) -> Result<MerkleProof, String> {
		debug!(LOGGER, "merkle_proof  {}, last_pos {}", pos, self.last_pos);

		// check this pos is actually a leaf in the MMR
		if !is_leaf(pos) {
			return Err(format!("not a leaf at pos {}", pos));
		}

		// check we actually have a hash in the MMR at this pos
		self.get_hash(pos)
			.ok_or(format!("no element at pos {}", pos))?;

		let mmr_size = self.unpruned_size();

		// Edge case: an MMR with a single entry in it
		// this entry is a leaf, a peak and the root itself
		// and there are no siblings to hash with
		if mmr_size == 1 {
			return Ok(MerkleProof {
				mmr_size,
				path: vec![],
			});
		}

		let family_branch = family_branch(pos, self.last_pos);

		let mut path = family_branch
			.iter()
			.filter_map(|x| self.get_from_file(x.1))
			.collect::<Vec<_>>();

		let peak_pos = match family_branch.last() {
			Some(&(x, _)) => x,
			None => pos,
		};

		path.append(&mut self.peak_path(peak_pos));

		Ok(MerkleProof { mmr_size, path })
	}

	/// Push a new element into the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push(&mut self, elmt: T) -> Result<u64, String> {
		let elmt_pos = self.last_pos + 1;
		let mut current_hash = elmt.hash_with_index(elmt_pos - 1);

		let mut to_append = vec![(current_hash, Some(elmt))];
		let mut height = 0;
		let mut pos = elmt_pos;

		// we look ahead one position in the MMR, if the expected node has a higher
		// height it means we have to build a higher peak by hashing with a previous
		// sibling. we do it iteratively in case the new peak itself allows the
		// creation of another parent.
		while bintree_postorder_height(pos + 1) > height {
			let left_sibling = bintree_jump_left_sibling(pos);

			let left_hash = self.backend
				.get_from_file(left_sibling)
				.ok_or("missing left sibling in tree, should not have been pruned")?;

			height += 1;
			pos += 1;

			current_hash = (left_hash, current_hash).hash_with_index(pos - 1);

			to_append.push((current_hash.clone(), None));
		}

		// append all the new nodes and update the MMR index
		self.backend.append(elmt_pos, to_append)?;
		self.last_pos = pos;
		Ok(elmt_pos)
	}

	/// Rewind the PMMR to a previous position, as if all push operations after
	/// that had been canceled. Expects a position in the PMMR to rewind to as
	/// well as the consumer-provided index of when the change occurred.
	pub fn rewind(&mut self, position: u64, index: u32) -> Result<(), String> {
		// identify which actual position we should rewind to as the provided
		// position is a leaf, which may had some parent that needs to exist
		// afterward for the MMR to be valid
		let mut pos = position;
		while bintree_postorder_height(pos + 1) > 0 {
			pos += 1;
		}

		self.backend.rewind(pos, index)?;
		self.last_pos = pos;
		Ok(())
	}

	/// Prune an element from the tree given its position. Note that to be able
	/// to provide that position and prune, consumers of this API are expected
	/// to keep an index of elements to positions in the tree. Prunes parent
	/// nodes as well when they become childless.
	pub fn prune(&mut self, position: u64, index: u32) -> Result<bool, String> {
		if let None = self.backend.get_hash(position) {
			return Ok(false);
		}
		let prunable_height = bintree_postorder_height(position);
		if prunable_height > 0 {
			// only leaves can be pruned
			return Err(format!("Node at {} is not a leaf, can't prune.", position));
		}

		// loop going up the tree, from node to parent, as long as we stay inside
		// the tree.
		let mut to_prune = vec![];

		let mut current = position;
		while current + 1 <= self.last_pos {
			let (parent, sibling) = family(current);

			to_prune.push(current);

			if parent > self.last_pos {
				// can't prune when our parent isn't here yet
				break;
			}

			// if we have a pruned sibling, we can continue up the tree
			// otherwise we're done
			if let None = self.backend.get_hash(sibling) {
				current = parent;
			} else {
				break;
			}
		}

		self.backend.remove(to_prune, index)?;
		Ok(true)
	}

	/// Get the hash at provided position in the MMR.
	pub fn get_hash(&self, pos: u64) -> Option<Hash> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_hash(pos)
		}
	}

	/// Get the data element at provided position in the MMR.
	pub fn get_data(&self, pos: u64) -> Option<T> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_data(pos)
		}
	}

	/// Get the hash from the underlying MMR file
	/// (ignores the remove log).
	fn get_from_file(&self, pos: u64) -> Option<Hash> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_from_file(pos)
		}
	}

	/// Helper function to get the last N nodes inserted, i.e. the last
	/// n nodes along the bottom of the tree
	pub fn get_last_n_insertions(&self, n: u64) -> Vec<(Hash, T)> {
		let mut return_vec = vec![];
		let mut last_leaf = self.last_pos;
		let size = self.unpruned_size();
		// Special case that causes issues in bintree functions,
		// just return
		if size == 1 {
			return_vec.push((
				self.backend.get_hash(last_leaf).unwrap(),
				self.backend.get_data(last_leaf).unwrap(),
			));
			return return_vec;
		}
		// if size is even, we're already at the bottom, otherwise
		// we need to traverse down to it (reverse post-order direction)
		if size % 2 == 1 {
			last_leaf = bintree_rightmost(self.last_pos);
		}
		for _ in 0..n as u64 {
			if last_leaf == 0 {
				break;
			}
			if bintree_postorder_height(last_leaf) > 0 {
				last_leaf = bintree_rightmost(last_leaf);
			}
			return_vec.push((
				self.backend.get_hash(last_leaf).unwrap(),
				self.backend.get_data(last_leaf).unwrap(),
			));

			last_leaf = bintree_jump_left_sibling(last_leaf);
		}
		return_vec
	}

	/// Helper function which returns un-pruned nodes from the insertion index
	/// forward
	/// returns last insertion index returned along with data
	pub fn elements_from_insertion_index(&self, mut index: u64, max_count: u64) -> (u64, Vec<T>) {
		let mut return_vec = vec![];
		if index == 0 {
			index = 1;
		}
		let mut return_index = index;
		let mut pmmr_index = insertion_to_pmmr_index(index);
		while return_vec.len() < max_count as usize && pmmr_index <= self.last_pos {
			if let Some(t) = self.get_data(pmmr_index) {
				return_vec.push(t);
				return_index = index;
			}
			index += 1;
			pmmr_index = insertion_to_pmmr_index(index);
		}
		(return_index, return_vec)
	}

	/// Walks all unpruned nodes in the MMR and revalidate all parent hashes
	pub fn validate(&self) -> Result<(), String> {
		// iterate on all parent nodes
		for n in 1..(self.last_pos + 1) {
			if bintree_postorder_height(n) > 0 {
				if let Some(hash) = self.get_hash(n) {
					// take the left and right children, if they exist
					let left_pos = bintree_move_down_left(n).ok_or(format!("left_pos not found"))?;
					let right_pos = bintree_jump_right_sibling(left_pos);

					// using get_from_file here for the children (they may have been "removed")
					if let Some(left_child_hs) = self.get_from_file(left_pos) {
						if let Some(right_child_hs) = self.get_from_file(right_pos) {
							// hash the two child nodes together with parent_pos and compare
							let (parent_pos, _) = family(left_pos);
							if (left_child_hs, right_child_hs).hash_with_index(parent_pos - 1)
								!= hash
							{
								return Err(format!(
									"Invalid MMR, hash of parent at {} does \
									 not match children.",
									n
								));
							}
						}
					}
				}
			}
		}
		Ok(())
	}

	/// Total size of the tree, including intermediary nodes and ignoring any
	/// pruning.
	pub fn unpruned_size(&self) -> u64 {
		self.last_pos
	}

	/// Return the path of the data file (needed to sum kernels efficiently)
	pub fn data_file_path(&self) -> String {
		self.backend.get_data_file_path()
	}

	/// Debugging utility to print information about the MMRs. Short version
	/// only prints the last 8 nodes.
	pub fn dump(&self, short: bool) {
		let sz = self.unpruned_size();
		if sz > 2000 && !short {
			return;
		}
		let start = if short && sz > 7 { sz / 8 - 1 } else { 0 };
		for n in start..(sz / 8 + 1) {
			let mut idx = "".to_owned();
			let mut hashes = "".to_owned();
			for m in (n * 8)..(n + 1) * 8 {
				if m >= sz {
					break;
				}
				idx.push_str(&format!("{:>8} ", m + 1));
				let ohs = self.get_hash(m + 1);
				match ohs {
					Some(hs) => hashes.push_str(&format!("{} ", hs)),
					None => hashes.push_str(&format!("{:>8} ", "??")),
				}
			}
			trace!(LOGGER, "{}", idx);
			trace!(LOGGER, "{}", hashes);
		}
	}

	/// Prints PMMR statistics to the logs, used for debugging.
	pub fn dump_stats(&self) {
		debug!(LOGGER, "pmmr: unpruned - {}", self.unpruned_size());
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
		let start = if short && sz > 7 { sz / 8 - 1 } else { 0 };
		for n in start..(sz / 8 + 1) {
			let mut idx = "".to_owned();
			let mut hashes = "".to_owned();
			for m in (n * 8)..(n + 1) * 8 {
				if m >= sz {
					break;
				}
				idx.push_str(&format!("{:>8} ", m + 1));
				let ohs = self.get_from_file(m + 1);
				match ohs {
					Some(hs) => hashes.push_str(&format!("{} ", hs)),
					None => hashes.push_str(&format!("{:>8} ", " .")),
				}
			}
			debug!(LOGGER, "{}", idx);
			debug!(LOGGER, "{}", hashes);
		}
	}
}

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

		match pruned_idx.or(next_idx) {
			None => None,
			Some(idx) => {
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
		}
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

/// Gets the postorder traversal index of all peaks in a MMR given the last
/// node's position. Starts with the top peak, which is always on the left
/// side of the range, and navigates toward lower siblings toward the right
/// of the range.
pub fn peaks(num: u64) -> Vec<u64> {
	if num == 0 {
		return vec![];
	}

	// detecting an invalid mountain range, when siblings exist but no parent
	// exists
	if bintree_postorder_height(num + 1) > bintree_postorder_height(num) {
		return vec![];
	}

	// our top peak is always on the leftmost side of the tree and leftmost trees
	// have for index a binary values with all 1s (i.e. 11, 111, 1111, etc.)
	let mut top = 1;
	while (top - 1) <= num {
		top = top << 1;
	}
	top = (top >> 1) - 1;
	if top == 0 {
		return vec![1];
	}

	let mut peaks = vec![top];

	// going down the range, next peaks are right neighbors of the top. if one
	// doesn't exist yet, we go down to a smaller peak to the left
	let mut peak = top;
	'outer: loop {
		peak = bintree_jump_right_sibling(peak);
		while peak > num {
			match bintree_move_down_left(peak) {
				Some(p) => peak = p,
				None => break 'outer,
			}
		}
		peaks.push(peak);
	}

	peaks
}

/// The number of leaves nodes in a MMR of the provided size. Uses peaks to
/// get the positions of all full binary trees and uses the height of these
pub fn n_leaves(mut sz: u64) -> u64 {
	if sz == 0 {
		return 0;
	}

	while bintree_postorder_height(sz + 1) > 0 {
		sz += 1;
	}
	peaks(sz)
		.iter()
		.map(|n| (1 << bintree_postorder_height(*n)) as u64)
		.sum()
}

/// Returns the pmmr index of the nth inserted element
pub fn insertion_to_pmmr_index(mut sz: u64) -> u64 {
	//1 based pmmrs
	sz = sz - 1;
	2 * sz - sz.count_ones() as u64 + 1
}

/// The height of a node in a full binary tree from its postorder traversal
/// index. This function is the base on which all others, as well as the MMR,
/// are built.
///
/// We first start by noticing that the insertion order of a node in a MMR [1]
/// is identical to the height of a node in a binary tree traversed in
/// postorder. Specifically, we want to be able to generate the following
/// sequence:
///
/// //    [0, 0, 1, 0, 0, 1, 2, 0, 0, 1, 0, 0, 1, 2, 3, 0, 0, 1, ...]
///
/// Which turns out to start as the heights in the (left, right, top)
/// -postorder- traversal of the following tree:
///
/// //               3
/// //             /   \
/// //           /       \
/// //         /           \
/// //        2             2
/// //      /  \          /  \
/// //     /    \        /    \
/// //    1      1      1      1
/// //   / \    / \    / \    / \
/// //  0   0  0   0  0   0  0   0
///
/// If we extend this tree up to a height of 4, we can continue the sequence,
/// and for an infinitely high tree, we get the infinite sequence of heights
/// in the MMR.
///
/// So to generate the MMR height sequence, we want a function that, given an
/// index in that sequence, gets us the height in the tree. This allows us to
/// build the sequence not only to infinite, but also at any index, without the
/// need to materialize the beginning of the sequence.
///
/// To see how to get the height of a node at any position in the postorder
/// traversal sequence of heights, we start by rewriting the previous tree with
/// each the position of every node written in binary:
///
///
/// //                  1111
/// //                 /   \
/// //               /       \
/// //             /           \
/// //           /               \
/// //        111                1110
/// //       /   \              /    \
/// //      /     \            /      \
/// //     11      110        1010     1101
/// //    / \      / \       /  \      / \
/// //   1   10  100  101  1000 1001 1011 1100
///
/// The height of a node is the number of 1 digits on the leftmost branch of
/// the tree, minus 1. For example, 1111 has 4 ones, so its height is `4-1=3`.
///
/// To get the height of any node (say 1101), we need to travel left in the
/// tree, get the leftmost node and count the ones. To travel left, we just
/// need to subtract the position by it's most significant bit, mins one. For
/// example to get from 1101 to 110 we subtract it by (1000-1) (`13-(8-1)=5`).
/// Then to to get 110 to 11, we subtract it by (100-1) ('6-(4-1)=3`).
///
/// By applying this operation recursively, until we get a number that, in
/// binary, is all ones, and then counting the ones, we can get the height of
/// any node, from its postorder traversal position. Which is the order in which
/// nodes are added in a MMR.
///
/// [1]  https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md
pub fn bintree_postorder_height(num: u64) -> u64 {
	let mut h = num;
	while !all_ones(h) {
		h = bintree_jump_left(h);
	}
	most_significant_pos(h) - 1
}

/// Is this position a leaf in the MMR?
/// We know the positions of all leaves based on the postorder height of an MMR of any size
/// (somewhat unintuitively but this is how the PMMR is "append only").
pub fn is_leaf(pos: u64) -> bool {
	if pos == 0 {
		return false;
	}
	bintree_postorder_height(pos) == 0
}

/// Calculates the positions of the parent and sibling of the node at the
/// provided position.
pub fn family(pos: u64) -> (u64, u64) {
	let pos_height = bintree_postorder_height(pos);
	let next_height = bintree_postorder_height(pos + 1);
	if next_height > pos_height {
		let sibling = bintree_jump_left_sibling(pos);
		let parent = pos + 1;
		(parent, sibling)
	} else {
		let sibling = bintree_jump_right_sibling(pos);
		let parent = sibling + 1;
		(parent, sibling)
	}
}

/// Is the node at this pos the "left" sibling of its parent?
pub fn is_left_sibling(pos: u64) -> bool {
	let (_, sibling_pos) = family(pos);
	sibling_pos > pos
}

/// For a given starting position calculate the parent and sibling positions
/// for the branch/path from that position to the peak of the tree.
/// We will use the sibling positions to generate the "path" of a Merkle proof.
pub fn family_branch(pos: u64, last_pos: u64) -> Vec<(u64, u64)> {
	// loop going up the tree, from node to parent, as long as we stay inside
	// the tree (as defined by last_pos).
	let mut branch = vec![];
	let mut current = pos;
	while current + 1 <= last_pos {
		let (parent, sibling) = family(current);
		if parent > last_pos {
			break;
		}
		branch.push((parent, sibling));

		current = parent;
	}
	branch
}

/// Calculates the position of the top-left child of a parent node in the
/// postorder traversal of a full binary tree.
fn bintree_move_down_left(num: u64) -> Option<u64> {
	let height = bintree_postorder_height(num);
	if height == 0 {
		return None;
	}
	Some(num - (1 << height))
}

/// Gets the position of the rightmost node (i.e. leaf) relative to the current
fn bintree_rightmost(num: u64) -> u64 {
	let height = bintree_postorder_height(num);
	if height == 0 {
		return 0;
	}
	num - height
}

/// Calculates the position of the right sibling of a node a subtree in the
/// postorder traversal of a full binary tree.
fn bintree_jump_right_sibling(num: u64) -> u64 {
	num + (1 << (bintree_postorder_height(num) + 1)) - 1
}

/// Calculates the position of the left sibling of a node a subtree in the
/// postorder traversal of a full binary tree.
fn bintree_jump_left_sibling(num: u64) -> u64 {
	num - ((1 << (bintree_postorder_height(num) + 1)) - 1)
}

/// Calculates the position of of a node to the left of the provided one when
/// jumping from the largest rightmost tree to its left equivalent in the
/// postorder traversal of a full binary tree.
fn bintree_jump_left(num: u64) -> u64 {
	num - ((1 << (most_significant_pos(num) - 1)) - 1)
}

/// Check if the binary representation of a number is all ones.
pub fn all_ones(num: u64) -> bool {
	let ones = num.count_ones();
	num.leading_zeros() + ones == 64 && ones > 0
}

/// Get the position of the most significant bit in a number.
pub fn most_significant_pos(num: u64) -> u64 {
	64 - u64::from(num.leading_zeros())
}
