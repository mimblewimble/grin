// Copyright 2017 The Grin Developers
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
//! 2. The implementation of a prunable MMR sum tree using the above. Each leaf
//! is required to be Summable and Hashed. Tree roots can be trivially and
//! efficiently calculated without materializing the full tree. The underlying
//! (Hash, Sum) pais are stored in a Backend implementation that can either be
//! a simple Vec or a database.

use std::clone::Clone;
use std::marker::PhantomData;
use std::ops::{self, Deref};

use core::hash::{Hash, Hashed};
use ser::{self, Readable, Reader, Writeable, Writer};
use util::LOGGER;

/// Trait for an element of the tree that has a well-defined sum and hash that
/// the tree can sum over
pub trait Summable {
	/// The type of the sum
	type Sum: Clone + ops::Add<Output = Self::Sum> + Readable + Writeable;

	/// Obtain the sum of the element
	fn sum(&self) -> Self::Sum;

	/// Length of the Sum type when serialized. Can be used as a hint by
	/// underlying storages.
	fn sum_len() -> usize;
}

/// An empty sum that takes no space, to store elements that do not need summing
/// but can still leverage the hierarchical hashing.
#[derive(Copy, Clone, Debug)]
pub struct NullSum;
impl ops::Add for NullSum {
	type Output = NullSum;
	fn add(self, _: NullSum) -> NullSum {
		NullSum
	}
}

impl Readable for NullSum {
	fn read(_: &mut Reader) -> Result<NullSum, ser::Error> {
		Ok(NullSum)
	}
}

impl Writeable for NullSum {
	fn write<W: Writer>(&self, _: &mut W) -> Result<(), ser::Error> {
		Ok(())
	}
}

/// Wrapper for a type that allows it to be inserted in a tree without summing
#[derive(Clone, Debug)]
pub struct NoSum<T>(pub T);
impl<T> Summable for NoSum<T> {
	type Sum = NullSum;
	fn sum(&self) -> NullSum {
		NullSum
	}
	fn sum_len() -> usize {
		return 0;
	}
}
impl<T> Writeable for NoSum<T>
where
	T: Writeable,
{
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.0.write(writer)
	}
}

/// A utility type to handle (Hash, Sum) pairs more conveniently. The addition
/// of two HashSums is the (Hash(h1|h2), h1 + h2) HashSum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashSum<T>
where
	T: Summable,
{
	/// The hash
	pub hash: Hash,
	/// The sum
	pub sum: T::Sum,
}

impl<T> HashSum<T>
where
	T: Summable + Hashed,
{
	/// Create a hash sum from a summable
	pub fn from_summable<W: Writeable>(idx: u64, elmt: &T, hash_with: Option<W>) -> HashSum<T> {
		let hash = match hash_with {
			Some(h) => elmt.hash_with(h),
			None => elmt.hash(),
		};
		let sum = elmt.sum();
		let node_hash = (idx, &sum, hash).hash();
		HashSum {
			hash: node_hash,
			sum: sum,
		}
	}
}

impl<T> Readable for HashSum<T>
where
	T: Summable,
{
	fn read(r: &mut Reader) -> Result<HashSum<T>, ser::Error> {
		Ok(HashSum {
			hash: Hash::read(r)?,
			sum: T::Sum::read(r)?,
		})
	}
}

impl<T> Writeable for HashSum<T>
where
	T: Summable,
{
	fn write<W: Writer>(&self, w: &mut W) -> Result<(), ser::Error> {
		self.hash.write(w)?;
		self.sum.write(w)
	}
}

impl<T> ops::Add for HashSum<T>
where
	T: Summable,
{
	type Output = HashSum<T>;
	fn add(self, other: HashSum<T>) -> HashSum<T> {
		HashSum {
			hash: (self.hash, other.hash).hash(),
			sum: self.sum + other.sum,
		}
	}
}

/// Storage backend for the MMR, just needs to be indexed by order of insertion.
/// The PMMR itself does not need the Backend to be accurate on the existence
/// of an element (i.e. remove could be a no-op) but layers above can
/// depend on an accurate Backend to check existence.
pub trait Backend<T>
where
	T: Summable,
{
	/// Append the provided HashSums to the backend storage. The position of the
	/// first element of the Vec in the MMR is provided to help the
	/// implementation.
	fn append(&mut self, position: u64, data: Vec<HashSum<T>>) -> Result<(), String>;

	/// Rewind the backend state to a previous position, as if all append
	/// operations after that had been canceled. Expects a position in the PMMR
	/// to rewind to as well as the consumer-provided index of when the change
	/// occurred (see remove).
	fn rewind(&mut self, position: u64, index: u32) -> Result<(), String>;

	/// Get a HashSum by insertion position
	fn get(&self, position: u64) -> Option<HashSum<T>>;

	/// Remove HashSums by insertion position. An index is also provided so the
	/// underlying backend can implement some rollback of positions up to a
	/// given index (practically the index is a the height of a block that
	/// triggered removal).
	fn remove(&mut self, positions: Vec<u64>, index: u32) -> Result<(), String>;
}

/// Prunable Merkle Mountain Range implementation. All positions within the tree
/// start at 1 as they're postorder tree traversal positions rather than array
/// indices.
///
/// Heavily relies on navigation operations within a binary tree. In particular,
/// all the implementation needs to keep track of the MMR structure is how far
/// we are in the sequence of nodes making up the MMR.
pub struct PMMR<'a, T, B>
where
	T: Summable,
	B: 'a + Backend<T>,
{
	last_pos: u64,
	backend: &'a mut B,
	// only needed for parameterizing Backend
	summable: PhantomData<T>,
}

impl<'a, T, B> PMMR<'a, T, B>
where
	T: Summable + Hashed + Clone,
	B: 'a + Backend<T>,
{
	/// Build a new prunable Merkle Mountain Range using the provided backend.
	pub fn new(backend: &'a mut B) -> PMMR<T, B> {
		PMMR {
			last_pos: 0,
			backend: backend,
			summable: PhantomData,
		}
	}

	/// Build a new prunable Merkle Mountain Range pre-initlialized until
	/// last_pos
	/// with the provided backend.
	pub fn at(backend: &'a mut B, last_pos: u64) -> PMMR<T, B> {
		PMMR {
			last_pos: last_pos,
			backend: backend,
			summable: PhantomData,
		}
	}

	/// Computes the root of the MMR. Find all the peaks in the current
	/// tree and "bags" them to get a single peak.
	pub fn root(&self) -> HashSum<T> {
		let peaks_pos = peaks(self.last_pos);
		let peaks: Vec<Option<HashSum<T>>> = map_vec!(peaks_pos, |&pi| self.backend.get(pi));

		let mut ret = None;
		for peak in peaks {
			ret = match (ret, peak) {
				(None, x) => x,
				(Some(hsum), None) => Some(hsum),
				(Some(lhsum), Some(rhsum)) => Some(lhsum + rhsum),
			}
		}
		ret.expect("no root, invalid tree")
	}

	/// Push a new Summable element in the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push<W: Writeable>(&mut self, elmt: T, hash_with: Option<W>) -> Result<u64, String> {
		let elmt_pos = self.last_pos + 1;
		let mut current_hashsum = HashSum::from_summable(elmt_pos, &elmt, hash_with);
		let mut to_append = vec![current_hashsum.clone()];
		let mut height = 0;
		let mut pos = elmt_pos;

		// we look ahead one position in the MMR, if the expected node has a higher
		// height it means we have to build a higher peak by summing with a previous
		// sibling. we do it iteratively in case the new peak itself allows the
		// creation of another parent.
		while bintree_postorder_height(pos + 1) > height {
			let left_sibling = bintree_jump_left_sibling(pos);
			let left_hashsum = self.backend.get(left_sibling).expect(
				"missing left sibling in tree, should not have been pruned",
			);
			current_hashsum = left_hashsum + current_hashsum;

			to_append.push(current_hashsum.clone());
			height += 1;
			pos += 1;
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
		if let None = self.backend.get(position) {
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
		while current + 1 < self.last_pos {
			let (parent, sibling) = family(current);
			if parent > self.last_pos {
				// can't prune when our parent isn't here yet
				break;
			}
			to_prune.push(current);

			// if we have a pruned sibling, we can continue up the tree
			// otherwise we're done
			if let None = self.backend.get(sibling) {
				current = parent;
			} else {
				break;
			}
		}

		self.backend.remove(to_prune, index)?;
		Ok(true)
	}

	/// Helper function to get the HashSum of a node at a given position from
	/// the backend.
	pub fn get(&self, position: u64) -> Option<HashSum<T>> {
		self.backend.get(position)
	}

	/// Helper function to get the last N nodes inserted, i.e. the last
	/// n nodes along the bottom of the tree
	pub fn get_last_n_insertions(&self, n: u64) -> Vec<HashSum<T>> {
		let mut return_vec = Vec::new();
		let mut last_leaf = self.last_pos;
		let size = self.unpruned_size();
		// Special case that causes issues in bintree functions,
		// just return
		if size == 1 {
			return_vec.push(self.backend.get(last_leaf).unwrap());
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
			return_vec.push(self.backend.get(last_leaf).unwrap());

			last_leaf = bintree_jump_left_sibling(last_leaf);
		}
		return_vec
	}

	/// Total size of the tree, including intermediary nodes an ignoring any
	/// pruning.
	pub fn unpruned_size(&self) -> u64 {
		self.last_pos
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
				let ohs = self.get(m + 1);
				match ohs {
					Some(hs) => hashes.push_str(&format!("{} ", hs.hash)),
					None => hashes.push_str(&format!("{:>8} ", "??")),
				}
			}
			debug!(LOGGER, "{}", idx);
			debug!(LOGGER, "{}", hashes);
		}
	}
}

/// Simple MMR backend implementation based on a Vector. Pruning does not
/// compact the Vector itself but still frees the reference to the
/// underlying HashSum.
#[derive(Clone)]
pub struct VecBackend<T>
where
	T: Summable + Clone,
{
	/// Backend elements
	pub elems: Vec<Option<HashSum<T>>>,
}

impl<T> Backend<T> for VecBackend<T>
where
	T: Summable + Clone,
{
	#[allow(unused_variables)]
	fn append(&mut self, position: u64, data: Vec<HashSum<T>>) -> Result<(), String> {
		self.elems.append(&mut map_vec!(data, |d| Some(d.clone())));
		Ok(())
	}
	fn get(&self, position: u64) -> Option<HashSum<T>> {
		self.elems[(position - 1) as usize].clone()
	}
	#[allow(unused_variables)]
	fn remove(&mut self, positions: Vec<u64>, index: u32) -> Result<(), String> {
		for n in positions {
			self.elems[(n - 1) as usize] = None
		}
		Ok(())
	}
	#[allow(unused_variables)]
	fn rewind(&mut self, position: u64, index: u32) -> Result<(), String> {
		self.elems = self.elems[0..(position as usize) + 1].to_vec();
		Ok(())
	}
}

impl<T> VecBackend<T>
where
	T: Summable + Clone,
{
	/// Instantiates a new VecBackend<T>
	pub fn new() -> VecBackend<T> {
		VecBackend { elems: vec![] }
	}

	/// Current number of HashSum elements in the underlying Vec.
	pub fn used_size(&self) -> usize {
		let mut usz = self.elems.len();
		for elem in self.elems.deref() {
			if elem.is_none() {
				usz -= 1;
			}
		}
		usz
	}

	/// Resets the backend, emptying the underlying Vec.
	pub fn clear(&mut self) {
		self.elems = Vec::new();
	}

	/// Total length of the underlying vector.
	pub fn len(&self) -> usize {
		self.elems.len()
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
		PruneList { pruned_nodes: vec![] }
	}

	/// Computes by how many positions a node at pos should be shifted given the
	/// number of nodes that have already been pruned before it.
	pub fn get_shift(&self, pos: u64) -> Option<u64> {
		// get the position where the node at pos would fit in the pruned list, if
		// it's already pruned, nothing to skip
		match self.pruned_pos(pos) {
			None => None,
			Some(idx) => {
				// skip by the number of elements pruned in the preceding subtrees,
				// which is the sum of the size of each subtree
				Some(
					self.pruned_nodes[0..(idx as usize)]
						.iter()
						.map(|n| (1 << (bintree_postorder_height(*n) + 1)) - 1)
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
					if let Err(idx) = self.pruned_nodes.binary_search(&current) {
						self.pruned_nodes.insert(idx, current);
					}
					break;
				}
			}
		}
	}

	/// Gets the position a new pruned node should take in the prune list.
	/// If the node has already bee pruned, either directly or through one of
	/// its parents contained in the prune list, returns None.
	pub fn pruned_pos(&self, pos: u64) -> Option<usize> {
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
fn peaks(num: u64) -> Vec<u64> {
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
/// [1]  https://github.
/// com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.
/// md
pub fn bintree_postorder_height(num: u64) -> u64 {
	let mut h = num;
	while !all_ones(h) {
		h = bintree_jump_left(h);
	}
	most_significant_pos(h) - 1
}

/// Calculates the positions of the parent and sibling of the node at the
/// provided position.
pub fn family(pos: u64) -> (u64, u64) {
	let sibling: u64;
	let parent: u64;

	let pos_height = bintree_postorder_height(pos);
	let next_height = bintree_postorder_height(pos + 1);
	if next_height > pos_height {
		sibling = bintree_jump_left_sibling(pos);
		parent = pos + 1;
	} else {
		sibling = bintree_jump_right_sibling(pos);
		parent = sibling + 1;
	}
	(parent, sibling)
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

// Check if the binary representation of a number is all ones.
fn all_ones(num: u64) -> bool {
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

// Get the position of the most significant bit in a number.
fn most_significant_pos(num: u64) -> u64 {
	let mut pos = 0;
	let mut bit = 1;
	while num >= bit {
		bit = bit << 1;
		pos += 1;
	}
	pos
}

#[cfg(test)]
mod test {
	use super::*;
	use core::hash::Hashed;

	#[test]
	fn some_all_ones() {
		for n in vec![1, 7, 255] {
			assert!(all_ones(n), "{} should be all ones", n);
		}
		for n in vec![6, 9, 128] {
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
	fn first_50_mmr_heights() {
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
	#[allow(unused_variables)]
	fn some_peaks() {
		let empty: Vec<u64> = vec![];
		assert_eq!(peaks(1), vec![1]);
		assert_eq!(peaks(2), empty);
		assert_eq!(peaks(3), vec![3]);
		assert_eq!(peaks(4), vec![3, 4]);
		assert_eq!(peaks(11), vec![7, 10, 11]);
		assert_eq!(peaks(22), vec![15, 22]);
		assert_eq!(peaks(32), vec![31, 32]);
		assert_eq!(peaks(35), vec![31, 34, 35]);
		assert_eq!(peaks(42), vec![31, 38, 41, 42]);
	}

	#[derive(Copy, Clone, Debug, PartialEq, Eq)]
	struct TestElem([u32; 4]);
	impl Summable for TestElem {
		type Sum = u64;
		fn sum(&self) -> u64 {
			// sums are not allowed to overflow, so we use this simple
			// non-injective "sum" function that will still be homomorphic
			self.0[0] as u64 * 0x1000 + self.0[1] as u64 * 0x100 + self.0[2] as u64 * 0x10 +
				self.0[3] as u64
		}
		fn sum_len() -> usize {
			8
		}
	}

	impl Writeable for TestElem {
		fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
			try!(writer.write_u32(self.0[0]));
			try!(writer.write_u32(self.0[1]));
			try!(writer.write_u32(self.0[2]));
			writer.write_u32(self.0[3])
		}
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
		pmmr.push(elems[0], None::<TestElem>).unwrap();
		let hash = Hashed::hash(&elems[0]);
		let sum = elems[0].sum();
		let node_hash = (1 as u64, &sum, hash).hash();
		assert_eq!(
			pmmr.root(),
			HashSum {
				hash: node_hash,
				sum: sum,
			}
		);
		assert_eq!(pmmr.unpruned_size(), 1);

		// two elements
		pmmr.push(elems[1], None::<TestElem>).unwrap();
		let sum2 = HashSum::from_summable(1, &elems[0], None::<TestElem>) +
			HashSum::from_summable(2, &elems[1], None::<TestElem>);
		assert_eq!(pmmr.root(), sum2);
		assert_eq!(pmmr.unpruned_size(), 3);

		// three elements
		pmmr.push(elems[2], None::<TestElem>).unwrap();
		let sum3 = sum2.clone() + HashSum::from_summable(4, &elems[2], None::<TestElem>);
		assert_eq!(pmmr.root(), sum3);
		assert_eq!(pmmr.unpruned_size(), 4);

		// four elements
		pmmr.push(elems[3], None::<TestElem>).unwrap();
		let sum4 = sum2 +
			(HashSum::from_summable(4, &elems[2], None::<TestElem>) +
				 HashSum::from_summable(5, &elems[3], None::<TestElem>));
		assert_eq!(pmmr.root(), sum4);
		assert_eq!(pmmr.unpruned_size(), 7);

		// five elements
		pmmr.push(elems[4], None::<TestElem>).unwrap();
		let sum5 = sum4.clone() + HashSum::from_summable(8, &elems[4], None::<TestElem>);
		assert_eq!(pmmr.root(), sum5);
		assert_eq!(pmmr.unpruned_size(), 8);

		// six elements
		pmmr.push(elems[5], None::<TestElem>).unwrap();
		let sum6 = sum4.clone() +
			(HashSum::from_summable(8, &elems[4], None::<TestElem>) +
				 HashSum::from_summable(9, &elems[5], None::<TestElem>));
		assert_eq!(pmmr.root(), sum6.clone());
		assert_eq!(pmmr.unpruned_size(), 10);

		// seven elements
		pmmr.push(elems[6], None::<TestElem>).unwrap();
		let sum7 = sum6 + HashSum::from_summable(11, &elems[6], None::<TestElem>);
		assert_eq!(pmmr.root(), sum7);
		assert_eq!(pmmr.unpruned_size(), 11);

		// eight elements
		pmmr.push(elems[7], None::<TestElem>).unwrap();
		let sum8 = sum4 +
			((HashSum::from_summable(8, &elems[4], None::<TestElem>) +
				  HashSum::from_summable(9, &elems[5], None::<TestElem>)) +
				 (HashSum::from_summable(11, &elems[6], None::<TestElem>) +
					  HashSum::from_summable(12, &elems[7], None::<TestElem>)));
		assert_eq!(pmmr.root(), sum8);
		assert_eq!(pmmr.unpruned_size(), 15);

		// nine elements
		pmmr.push(elems[8], None::<TestElem>).unwrap();
		let sum9 = sum8 + HashSum::from_summable(16, &elems[8], None::<TestElem>);
		assert_eq!(pmmr.root(), sum9);
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
			TestElem([0, 0, 0, 9]),
		];
		let mut ba = VecBackend::new();
		let mut pmmr = PMMR::new(&mut ba);

		// test when empty
		let res = pmmr.get_last_n_insertions(19);
		assert!(res.len() == 0);

		pmmr.push(elems[0], None::<TestElem>).unwrap();
		let res = pmmr.get_last_n_insertions(19);
		assert!(res.len() == 1 && res[0].sum == 1);

		pmmr.push(elems[1], None::<TestElem>).unwrap();

		let res = pmmr.get_last_n_insertions(12);
		assert!(res[0].sum == 2 && res[1].sum == 1);

		pmmr.push(elems[2], None::<TestElem>).unwrap();

		let res = pmmr.get_last_n_insertions(2);
		assert!(res[0].sum == 3 && res[1].sum == 2);

		pmmr.push(elems[3], None::<TestElem>).unwrap();

		let res = pmmr.get_last_n_insertions(19);
		assert!(
			res[0].sum == 4 && res[1].sum == 3 && res[2].sum == 2 && res[3].sum == 1 && res.len() == 4
		);

		pmmr.push(elems[5], None::<TestElem>).unwrap();
		pmmr.push(elems[6], None::<TestElem>).unwrap();
		pmmr.push(elems[7], None::<TestElem>).unwrap();
		pmmr.push(elems[8], None::<TestElem>).unwrap();

		let res = pmmr.get_last_n_insertions(7);
		assert!(
			res[0].sum == 9 && res[1].sum == 8 && res[2].sum == 7 && res[3].sum == 6 && res.len() == 7
		);
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

		let orig_root: HashSum<TestElem>;
		let sz: u64;
		let mut ba = VecBackend::new();
		{
			let mut pmmr = PMMR::new(&mut ba);
			for elem in &elems[..] {
				pmmr.push(*elem, None::<TestElem>).unwrap();
			}
			orig_root = pmmr.root();
			sz = pmmr.unpruned_size();
		}

		// pruning a leaf with no parent should do nothing
		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			pmmr.prune(16, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 16);

		// pruning leaves with no shared parent just removes 1 element
		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			pmmr.prune(2, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 15);

		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			pmmr.prune(4, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 14);

		// pruning a non-leaf node has no effect
		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			pmmr.prune(3, 0).unwrap_err();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 14);

		// pruning sibling removes subtree
		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			pmmr.prune(5, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 12);

		// pruning all leaves under level >1 removes all subtree
		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			pmmr.prune(1, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 9);

		// pruning everything should only leave us the peaks
		{
			let mut pmmr = PMMR::at(&mut ba, sz);
			for n in 1..16 {
				let _ = pmmr.prune(n, 0);
			}
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 2);
	}

	#[test]
	fn pmmr_prune_list() {
		let mut pl = PruneList::new();
		pl.add(4);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes[0], 4);
		assert_eq!(pl.get_shift(5), Some(1));
		assert_eq!(pl.get_shift(2), Some(0));
		assert_eq!(pl.get_shift(4), None);

		pl.add(5);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes[0], 6);
		assert_eq!(pl.get_shift(8), Some(3));
		assert_eq!(pl.get_shift(2), Some(0));
		assert_eq!(pl.get_shift(5), None);

		pl.add(2);
		assert_eq!(pl.pruned_nodes.len(), 2);
		assert_eq!(pl.pruned_nodes[0], 2);
		assert_eq!(pl.get_shift(8), Some(4));
		assert_eq!(pl.get_shift(1), Some(0));

		pl.add(8);
		pl.add(11);
		assert_eq!(pl.pruned_nodes.len(), 4);

		pl.add(1);
		assert_eq!(pl.pruned_nodes.len(), 3);
		assert_eq!(pl.pruned_nodes[0], 7);
		assert_eq!(pl.get_shift(12), Some(9));

		pl.add(12);
		assert_eq!(pl.pruned_nodes.len(), 3);
		assert_eq!(pl.get_shift(12), None);
		assert_eq!(pl.get_shift(9), Some(8));
		assert_eq!(pl.get_shift(17), Some(11));
	}

}
