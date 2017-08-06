// Copyright 2016 The Grin Developers
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
use std::fmt::Debug;
use std::marker::PhantomData;
use std::ops::{self, Deref};

use core::hash::{Hash, Hashed};
use ser::{self, Readable, Reader, Writeable, Writer};

/// Trait for an element of the tree that has a well-defined sum and hash that
/// the tree can sum over
pub trait Summable {
	/// The type of the sum
	type Sum: Clone + ops::Add<Output = Self::Sum> + Readable + Writeable;

	/// Obtain the sum of the element
	fn sum(&self) -> Self::Sum;

	/// Length of the Sum type when serialized. Can be used as a hint by
	/// underlying storages.
	fn sum_len(&self) -> usize;
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
pub struct NoSum<T>(T);
impl<T> Summable for NoSum<T> {
	type Sum = NullSum;
	fn sum(&self) -> NullSum {
		NullSum
	}
	fn sum_len(&self) -> usize {
		return 0;
	}
}

/// A utility type to handle (Hash, Sum) pairs more conveniently. The addition
/// of two HashSums is the (Hash(h1|h2), h1 + h2) HashSum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashSum<T> where T: Summable {
	pub hash: Hash,
	pub sum: T::Sum,
}

impl<T> HashSum<T> where T: Summable + Writeable {
	pub fn from_summable(idx: u64, elmt: T) -> HashSum<T> {
		let hash = Hashed::hash(&elmt);
		let sum = elmt.sum();
		let node_hash = (idx, &sum, hash).hash();
		HashSum {
			hash: node_hash,
			sum: sum,
		}
	}
}

impl<T> Readable for HashSum<T> where T: Summable {
	fn read(r: &mut Reader) -> Result<HashSum<T>, ser::Error> {
		Ok(HashSum {
			hash: Hash::read(r)?,
			sum: T::Sum::read(r)?,
		})
	}
}

impl<T> Writeable for HashSum<T> where T: Summable {
	fn write<W: Writer>(&self, w: &mut W) -> Result<(), ser::Error> {
		self.hash.write(w)?;
		self.sum.write(w)
	}
}

impl<T> ops::Add for HashSum<T> where T: Summable {
	type Output = HashSum<T>;
	fn add(self, other: HashSum<T>) -> HashSum<T> {
		HashSum {
			hash: (self.hash, other.hash).hash(),
			sum: self.sum + other.sum,
		}
	}
}

/// Storage backend for the MMR, just needs to be indexed by order of insertion.
/// The remove operation can be a no-op for unoptimized backends.
pub trait Backend<T> where T: Summable {
	/// Append the provided HashSums to the backend storage.
	fn append(&self, data: Vec<HashSum<T>>);
	/// Get a HashSum by insertion position
	fn get(&self, position: u64) -> Option<HashSum<T>>;
	/// Remove HashSums by insertion position
	fn remove(&self, positions: Vec<u64>);
}

/// Prunable Merkle Mountain Range implementation. All positions within the tree
/// start at 1 as they're postorder tree traversal positions rather than array
/// indices.
///
/// Heavily relies on navigation operations within a binary tree. In particular,
/// all the implementation needs to keep track of the MMR structure is how far
/// we are in the sequence of nodes making up the MMR.
struct PMMR<T, B> where T: Summable, B: Backend<T> {
	last_pos: u64,
	backend: B,
	// only needed for parameterizing Backend
	summable: PhantomData<T>,
}

impl<T, B> PMMR<T, B> where T: Summable + Writeable + Debug + Clone, B: Backend<T> {

	/// Build a new prunable Merkle Mountain Range using the provided backend.
	pub fn new(backend: B) -> PMMR<T, B> {
		PMMR {
			last_pos: 0,
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
				(Some(lhsum), Some(rhsum)) => Some(lhsum + rhsum)
			}
		}
		ret.expect("no root, invalid tree")
	}

	/// Push a new Summable element in the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push(&mut self, elmt: T) -> u64 {
		let elmt_pos = self.last_pos + 1;
		let mut current_hashsum = HashSum::from_summable(elmt_pos, elmt);
		let mut to_append = vec![current_hashsum.clone()];
		let mut height = 0;
		let mut pos = elmt_pos;
		
		// we look ahead one position in the MMR, if the expected node has a higher
		// height it means we have to build a higher peak by summing with a previous
		// sibling. we do it iteratively in case the new peak itself allows the
		// creation of another parent.
		while bintree_postorder_height(pos+1) > height {
			let left_sibling = bintree_jump_left_sibling(pos);
			let left_hashsum = self.backend.get(left_sibling)
				.expect("missing left sibling in tree, should not have been pruned");
			current_hashsum = left_hashsum + current_hashsum;

			to_append.push(current_hashsum.clone());
			height += 1;
			pos += 1;
		}

		// append all the new nodes and update the MMR index
		self.backend.append(to_append);
		self.last_pos = pos;
		elmt_pos
	}

	/// Prune an element from the tree given its index. Note that to be able to
	/// provide that position and prune, consumers of this API are expected to
	/// keep an index of elements to positions in the tree. Prunes parent
	/// nodes as well when they become childless.
	pub fn prune(&self, position: u64) {
		let prunable_height = bintree_postorder_height(position);
		if prunable_height > 0 {
			// only leaves can be pruned
			return;
		}
	
		// loop going up the tree, from node to parent, as long as we stay inside
		// the tree.
		let mut to_prune = vec![];
		let mut current = position;
		while current+1 < self.last_pos {
			let current_height = bintree_postorder_height(current);
			let next_height = bintree_postorder_height(current+1);

			// compare the node's height to the next height, if the next is higher
			// we're on the right hand side of the subtree (otherwise we're on the
			// left)
			let sibling: u64;
			let parent: u64;
			if next_height > prunable_height {
				sibling = bintree_jump_left_sibling(current);
				parent = current + 1;
			} else {
				sibling = bintree_jump_right_sibling(current);
				parent = sibling + 1;
			}

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

		self.backend.remove(to_prune);
	}

	/// Total size of the tree, including intermediary nodes an ignoring any
	/// pruning.
	pub fn unpruned_size(&self) -> u64 {
		self.last_pos
	}
}

/// Gets the postorder traversal index of all peaks in a MMR given the last
/// node's position. Starts with the top peak, which is always on the left
/// side of the range, and navigates toward lower siblings toward the right
/// of the range.
fn peaks(num: u64) -> Vec<u64> {

	// detecting an invalid mountain range, when siblings exist but no parent
	// exists
	if bintree_postorder_height(num+1) > bintree_postorder_height(num) {
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
		//println!("peak {}", peak);
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
/// [1]  https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md
fn bintree_postorder_height(num: u64) -> u64 {
	let mut h = num;
	while !all_ones(h) {
		h = bintree_jump_left(h);
	}
	most_significant_pos(h) - 1
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
	use core::hash::{Hash, Hashed};
	use std::sync::{Arc, Mutex};

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
	fn first_50_mmr_heights() {
		let first_100_str =
			"0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 \
			0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 5 \
			0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3 4 0 0 1 0 0";
		let first_100 = first_100_str.split(' ').map(|n| n.parse::<u64>().unwrap());
		let mut count = 1;
		for n in first_100 {
			assert_eq!(n, bintree_postorder_height(count), "expected {}, got {}",
				n, bintree_postorder_height(count));
			count += 1;
		}
	}

	#[test]
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
		fn sum_len(&self) -> usize {
			4
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

	#[derive(Clone)]
	struct VecBackend {
		elems: Arc<Mutex<Vec<Option<HashSum<TestElem>>>>>,
	}
	impl Backend<TestElem> for VecBackend {
		fn append(&self, data: Vec<HashSum<TestElem>>) {
			let mut elems = self.elems.lock().unwrap();
			elems.append(&mut map_vec!(data, |d| Some(d.clone())));
		}
		fn get(&self, position: u64) -> Option<HashSum<TestElem>> {
			let elems = self.elems.lock().unwrap();
			elems[(position-1) as usize].clone()
		}
		fn remove(&self, positions: Vec<u64>) {
			let mut elems = self.elems.lock().unwrap();
			for n in positions {
				elems[(n-1) as usize] = None
			}
		}
	}
	impl VecBackend {
		fn used_size(&self) -> usize {
			let elems = self.elems.lock().unwrap();
			let mut usz = elems.len();
			for elem in elems.deref() {
				if elem.is_none() {
					usz -= 1;
				}
			}
			usz
		}
	}

	#[test]
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

		let ba = VecBackend{elems: Arc::new(Mutex::new(vec![]))};
		let mut pmmr = PMMR::new(ba.clone());

		// one element
		pmmr.push(elems[0]);
		let hash = Hashed::hash(&elems[0]);
		let sum = elems[0].sum();
		let node_hash = (1 as u64, &sum, hash).hash();
		assert_eq!(pmmr.root(), HashSum{hash: node_hash, sum: sum});
		assert_eq!(pmmr.unpruned_size(), 1);

		// two elements
		pmmr.push(elems[1]);
		let sum2 = HashSum::from_summable(1, elems[0]) + HashSum::from_summable(2, elems[1]);
		assert_eq!(pmmr.root(), sum2);
		assert_eq!(pmmr.unpruned_size(), 3);

		// three elements
		pmmr.push(elems[2]);
		let sum3 = sum2.clone() + HashSum::from_summable(4, elems[2]);
		assert_eq!(pmmr.root(), sum3);
		assert_eq!(pmmr.unpruned_size(), 4);

		// four elements
		pmmr.push(elems[3]);
		let sum4 = sum2 + (HashSum::from_summable(4, elems[2]) + HashSum::from_summable(5, elems[3]));
		assert_eq!(pmmr.root(), sum4);
		assert_eq!(pmmr.unpruned_size(), 7);

		// five elements
		pmmr.push(elems[4]);
		let sum5 = sum4.clone() + HashSum::from_summable(8, elems[4]);
		assert_eq!(pmmr.root(), sum5);
		assert_eq!(pmmr.unpruned_size(), 8);

		// six elements
		pmmr.push(elems[5]);
		let sum6 = sum4.clone() + (HashSum::from_summable(8, elems[4]) + HashSum::from_summable(9, elems[5]));
		assert_eq!(pmmr.root(), sum6.clone());
		assert_eq!(pmmr.unpruned_size(), 10);

		// seven elements
		pmmr.push(elems[6]);
		let sum7 = sum6 + HashSum::from_summable(11, elems[6]);
		assert_eq!(pmmr.root(), sum7);
		assert_eq!(pmmr.unpruned_size(), 11);

		// eight elements
		pmmr.push(elems[7]);
		let sum8 = sum4 + ((HashSum::from_summable(8, elems[4]) + HashSum::from_summable(9, elems[5])) + (HashSum::from_summable(11, elems[6]) + HashSum::from_summable(12, elems[7])));
		assert_eq!(pmmr.root(), sum8);
		assert_eq!(pmmr.unpruned_size(), 15);

		// nine elements
		pmmr.push(elems[8]);
		let sum9 = sum8 + HashSum::from_summable(16, elems[8]);
		assert_eq!(pmmr.root(), sum9);
		assert_eq!(pmmr.unpruned_size(), 16);
	}

	#[test]
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

		let ba = VecBackend{elems: Arc::new(Mutex::new(vec![]))};
		let mut pmmr = PMMR::new(ba.clone());
		for elem in &elems[..] {
			pmmr.push(*elem);
		}
		let orig_root = pmmr.root();
		let orig_sz = ba.used_size();

		// pruning a leaf with no parent should do nothing
		pmmr.prune(16);
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), orig_sz);

		// pruning leaves with no shared parent just removes 1 element
		pmmr.prune(2);
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), orig_sz - 1);

		pmmr.prune(4);
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), orig_sz - 2);

		// pruning a non-leaf node has no effect
		pmmr.prune(3);
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), orig_sz - 2);

		// pruning sibling removes subtree
		pmmr.prune(5);
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), orig_sz - 4);

		// pruning all leaves under level >1 removes all subtree
		pmmr.prune(1);
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), orig_sz - 7);
		
		// pruning everything should only leave us the peaks
		for n in 1..16 {
			pmmr.prune(n);
		}
		assert_eq!(orig_root, pmmr.root());
		assert_eq!(ba.used_size(), 2);
	}
}
