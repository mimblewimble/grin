// Copyright 2020 The Grin Developers
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

use std::marker;
use std::u64;

use croaring::Bitmap;

use crate::core::hash::{DefaultHashable, Hash, ZERO_HASH};
use crate::core::merkle_proof::MerkleProof;
use crate::core::pmmr::{Backend, ReadonlyPMMR};
use crate::core::BlockHeader;
use crate::ser::{HashEntry, PMMRIndexHashable, PMMRable};

/// 64 bits all ones: 0b11111111...1
const ALL_ONES: u64 = u64::MAX;

/// Trait with common methods for reading from a PMMR
pub trait ReadablePMMR {
	/// Leaf type
	type Item;
	/// Hash type
	type H: HashEntry + Default;

	/// Get the hash at provided position in the MMR.
	fn get_hash(&self, pos: u64) -> Option<Self::H>;

	/// Get the data element at provided position in the MMR.
	fn get_data(&self, pos: u64) -> Option<Self::Item>;

	/// Get the hash from the underlying MMR file (ignores the remove log).
	fn get_from_file(&self, pos: u64) -> Option<Self::H>;

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

	fn hash_children(index: u64, lc: Self::H, rc: Self::H) -> Self::H;

	/// Is the MMR empty?
	fn is_empty(&self) -> bool {
		self.unpruned_size() == 0
	}

	/// Takes a single peak position and hashes together
	/// all the peaks to the right of this peak (if any).
	/// If this return a hash then this is our peaks sibling.
	/// If none then the sibling of our peak is the peak to the left.
	fn bag_the_rhs(&self, peak_pos: u64) -> Option<Self::H> {
		let last_pos = self.unpruned_size();
		let rhs = peaks(last_pos)
			.into_iter()
			.filter(|&x| x > peak_pos)
			.filter_map(|x| self.get_from_file(x));

		let mut res = None;
		for peak in rhs.rev() {
			res = match res {
				None => Some(peak),
				Some(rhash) => Some(Self::hash_children(last_pos, peak, rhash)),
			}
		}
		res
	}

	/// Returns a vec of the peaks of this MMR.
	fn peaks(&self) -> Vec<Self::H> {
		peaks(self.unpruned_size())
			.into_iter()
			.filter_map(move |pi| {
				// here we want to get from underlying hash file
				// as the pos *may* have been "removed"
				self.get_from_file(pi)
			})
			.collect()
	}

	/// Hashes of the peaks excluding `peak_pos`, where the rhs is bagged together
	fn peak_path(&self, peak_pos: u64) -> Vec<Self::H> {
		let rhs = self.bag_the_rhs(peak_pos);
		let mut res = peaks(self.unpruned_size())
			.into_iter()
			.filter(|&x| x < peak_pos)
			.filter_map(|x| self.get_from_file(x))
			.collect::<Vec<_>>();
		if let Some(rhs) = rhs {
			res.push(rhs);
		}
		res.reverse();

		res
	}

	/// Computes the root of the MMR. Find all the peaks in the current
	/// tree and "bags" them to get a single peak.
	fn root(&self) -> Result<Self::H, String> {
		if self.is_empty() {
			return Ok(Default::default());
		}
		let mut res = None;
		let peaks = self.peaks();
		for peak in peaks.into_iter().rev() {
			res = match res {
				None => Some(peak),
				Some(rhash) => Some(Self::hash_children(self.unpruned_size(), peak, rhash)), // Some((peak, rhash).hash_with_index(self.unpruned_size())),
			}
		}
		res.ok_or_else(|| "no root, invalid tree".to_owned())
	}
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
	T: PMMRable + PMMRIndexHashable,
	B: Backend<T>,
{
	/// The last position in the PMMR
	pub last_pos: u64,
	backend: &'a mut B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> PMMR<'a, T, B>
where
	T: PMMRable + PMMRIndexHashable,
	B: 'a + Backend<T>,
{
	/// Build a new prunable Merkle Mountain Range using the provided backend.
	pub fn new(backend: &'a mut B) -> PMMR<'_, T, B> {
		PMMR {
			backend,
			last_pos: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new prunable Merkle Mountain Range pre-initialized until
	/// last_pos with the provided backend.
	pub fn at(backend: &'a mut B, last_pos: u64) -> PMMR<'_, T, B> {
		PMMR {
			backend,
			last_pos,
			_marker: marker::PhantomData,
		}
	}

	/// Build a "readonly" view of this PMMR.
	pub fn readonly_pmmr(&self) -> ReadonlyPMMR<'_, T, B> {
		ReadonlyPMMR::at(&self.backend, self.last_pos)
	}

	/// Push a new element into the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push(&mut self, elmt: &T) -> Result<u64, String> {
		let elmt_pos = self.last_pos + 1;
		let mut current_hash = elmt.hash_with_index(elmt_pos - 1);

		let mut hashes = vec![current_hash];
		let mut pos = elmt_pos;

		let (peak_map, height) = peak_map_height(pos - 1);
		if height != 0 {
			return Err(format!("bad mmr size {}", pos - 1));
		}
		// hash with all immediately preceding peaks, as indicated by peak map
		let mut peak = 1;
		while (peak_map & peak) != 0 {
			let left_sibling = pos + 1 - 2 * peak;
			let left_hash = self
				.backend
				.get_from_file(left_sibling)
				.ok_or("missing left sibling in tree, should not have been pruned")?;
			peak *= 2;
			pos += 1;
			current_hash = T::hash_children(pos - 1, left_hash, current_hash);
			hashes.push(current_hash);
		}

		// append all the new nodes and update the MMR index
		self.backend.append(elmt, hashes)?;
		self.last_pos = pos;
		Ok(elmt_pos)
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
		let mut pos = position;
		while bintree_postorder_height(pos + 1) > 0 {
			pos += 1;
		}

		self.backend.rewind(pos, rewind_rm_pos)?;
		self.last_pos = pos;
		Ok(())
	}

	/// Prunes (removes) the leaf from the MMR at the specified position.
	/// Returns an error if prune is called on a non-leaf position.
	/// Returns false if the leaf node has already been pruned.
	/// Returns true if pruning is successful.
	pub fn prune(&mut self, position: u64) -> Result<bool, String> {
		if !is_leaf(position) {
			return Err(format!("Node at {} is not a leaf, can't prune.", position));
		}

		if self.backend.get_hash(position).is_none() {
			return Ok(false);
		}

		self.backend.remove(position)?;
		Ok(true)
	}

	/// Walks all unpruned nodes in the MMR and revalidate all parent hashes
	pub fn validate(&self) -> Result<(), String> {
		// iterate on all parent nodes
		for n in 1..(self.last_pos + 1) {
			let height = bintree_postorder_height(n);
			if height > 0 {
				if let Some(hash) = self.get_hash(n) {
					let left_pos = n - (1 << height);
					let right_pos = n - 1;
					// using get_from_file here for the children (they may have been "removed")
					if let Some(left_child_hs) = self.get_from_file(left_pos) {
						if let Some(right_child_hs) = self.get_from_file(right_pos) {
							// hash the two child nodes together with parent_pos and compare
							if T::hash_children(n - 1, left_child_hs, right_child_hs) != hash {
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
					Some(hs) => hashes.push_str(&format!("{} ", hs.as_hash())),
					None => hashes.push_str(&format!("{:>8} ", "??")),
				}
			}
			trace!("{}", idx);
			trace!("{}", hashes);
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
					Some(hs) => hashes.push_str(&format!("{} ", hs.as_hash())),
					None => hashes.push_str(&format!("{:>8} ", " .")),
				}
			}
			debug!("{}", idx);
			debug!("{}", hashes);
		}
	}

	/// Build a Merkle proof for the element at the given position.
	///
	///
	fn merkle_proof(&self, pos: u64) -> Result<MerkleProof<T>, String> {
		let last_pos = self.unpruned_size();
		debug!("merkle_proof  {}, last_pos {}", pos, last_pos);

		// check this pos is actually a leaf in the MMR
		if !is_leaf(pos) {
			return Err(format!("not a leaf at pos {}", pos));
		}

		// check we actually have a hash in the MMR at this pos
		self.get_hash(pos)
			.ok_or_else(|| format!("no element at pos {}", pos))?;

		let family_branch = family_branch(pos, last_pos);

		let mut path = family_branch
			.iter()
			.filter_map(|x| self.get_from_file(x.1))
			.collect::<Vec<_>>();

		let peak_pos = match family_branch.last() {
			Some(&(x, _)) => x,
			None => pos,
		};

		path.append(&mut self.peak_path(peak_pos));

		Ok(MerkleProof {
			mmr_size: last_pos,
			path,
		})
	}
}

impl<'a, T, B> ReadablePMMR for PMMR<'a, T, B>
where
	T: PMMRable + PMMRIndexHashable,
	B: 'a + Backend<T>,
{
	type Item = T::E;
	type H = T::H;

	fn get_hash(&self, pos: u64) -> Option<T::H> {
		if pos > self.last_pos {
			None
		} else if is_leaf(pos) {
			// If we are a leaf then get hash from the backend.
			self.backend.get_hash(pos)
		} else {
			// If we are not a leaf get hash ignoring the remove log.
			self.backend.get_from_file(pos)
		}
	}

	fn get_data(&self, pos: u64) -> Option<Self::Item> {
		if pos > self.last_pos {
			// If we are beyond the rhs of the MMR return None.
			None
		} else if is_leaf(pos) {
			// If we are a leaf then get data from the backend.
			self.backend.get_data(pos)
		} else {
			// If we are not a leaf then return None as only leaves have data.
			None
		}
	}

	fn get_from_file(&self, pos: u64) -> Option<T::H> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_from_file(pos)
		}
	}

	fn get_data_from_file(&self, pos: u64) -> Option<Self::Item> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_data_from_file(pos)
		}
	}

	fn unpruned_size(&self) -> u64 {
		self.last_pos
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

	// Delegate child hashing logic.
	fn hash_children(index: u64, lc: Self::H, rc: Self::H) -> Self::H {
		T::hash_children(index, lc, rc)
	}
}

/// Gets the postorder traversal index of all peaks in a MMR given its size.
/// Starts with the top peak, which is always on the left
/// side of the range, and navigates toward lower siblings toward the right
/// of the range.
pub fn peaks(num: u64) -> Vec<u64> {
	if num == 0 {
		return vec![];
	}
	let mut peak_size = ALL_ONES >> num.leading_zeros();
	let mut num_left = num;
	let mut sum_prev_peaks = 0;
	let mut peaks = vec![];
	while peak_size != 0 {
		if num_left >= peak_size {
			peaks.push(sum_prev_peaks + peak_size);
			sum_prev_peaks += peak_size;
			num_left -= peak_size;
		}
		peak_size >>= 1;
	}
	if num_left > 0 {
		return vec![];
	}
	peaks
}

/// The number of leaves in a MMR of the provided size.
pub fn n_leaves(size: u64) -> u64 {
	let (sizes, height) = peak_sizes_height(size);
	let nleaves = sizes.into_iter().map(|n| (n + 1) / 2 as u64).sum();
	if height == 0 {
		nleaves
	} else {
		nleaves + 1
	}
}

/// Returns the pmmr index of the nth inserted element
pub fn insertion_to_pmmr_index(mut sz: u64) -> u64 {
	if sz == 0 {
		return 0;
	}
	// 1 based pmmrs
	sz -= 1;
	2 * sz - sz.count_ones() as u64 + 1
}

/// sizes of peaks and height of next node in mmr of given size
/// Example: on input 5 returns ([3,1], 1) as mmr state before adding 5 was
///    2
///   / \
///  0   1   3   4
pub fn peak_sizes_height(size: u64) -> (Vec<u64>, u64) {
	if size == 0 {
		return (vec![], 0);
	}
	let mut peak_size = ALL_ONES >> size.leading_zeros();
	let mut sizes = vec![];
	let mut size_left = size;
	while peak_size != 0 {
		if size_left >= peak_size {
			sizes.push(peak_size);
			size_left -= peak_size;
		}
		peak_size >>= 1;
	}
	(sizes, size_left)
}

/// return (peak_map, pos_height) of given 0-based node pos prior to its
/// addition
/// Example: on input 4 returns (0b11, 0) as mmr state before adding 4 was
///    2
///   / \
///  0   1   3
/// with 0b11 indicating presence of peaks of height 0 and 1.
/// NOTE:
/// the peak map also encodes the path taken from the root to the added node
/// since the path turns left (resp. right) if-and-only-if
/// a peak at that height is absent (resp. present)
pub fn peak_map_height(mut pos: u64) -> (u64, u64) {
	if pos == 0 {
		return (0, 0);
	}
	let mut peak_size = ALL_ONES >> pos.leading_zeros();
	let mut bitmap = 0;
	while peak_size != 0 {
		bitmap <<= 1;
		if pos >= peak_size {
			pos -= peak_size;
			bitmap |= 1;
		}
		peak_size >>= 1;
	}
	(bitmap, pos)
}

/// The height of a node in a full binary tree from its postorder traversal
/// index. This function is the base on which all others, as well as the MMR,
/// are built.
pub fn bintree_postorder_height(num: u64) -> u64 {
	if num == 0 {
		return 0;
	}
	peak_map_height(num - 1).1
}

/// Is this position a leaf in the MMR?
/// We know the positions of all leaves based on the postorder height of an MMR
/// of any size (somewhat unintuitively but this is how the PMMR is "append
/// only").
pub fn is_leaf(pos: u64) -> bool {
	bintree_postorder_height(pos) == 0
}

/// Calculates the positions of the parent and sibling of the node at the
/// provided position.
pub fn family(pos: u64) -> (u64, u64) {
	let (peak_map, height) = peak_map_height(pos - 1);
	let peak = 1 << height;
	if (peak_map & peak) != 0 {
		(pos + 1, pos + 1 - 2 * peak)
	} else {
		(pos + 2 * peak, pos + 2 * peak - 1)
	}
}

/// Is the node at this pos the "left" sibling of its parent?
pub fn is_left_sibling(pos: u64) -> bool {
	let (peak_map, height) = peak_map_height(pos - 1);
	let peak = 1 << height;
	(peak_map & peak) == 0
}

/// Returns the path from the specified position up to its
/// corresponding peak in the MMR.
/// The size (and therefore the set of peaks) of the MMR
/// is defined by last_pos.
pub fn path(pos: u64, last_pos: u64) -> impl Iterator<Item = u64> {
	Path::new(pos, last_pos)
}

struct Path {
	current: u64,
	last_pos: u64,
	peak: u64,
	peak_map: u64,
}

impl Path {
	fn new(pos: u64, last_pos: u64) -> Self {
		let (peak_map, height) = peak_map_height(pos - 1);
		Path {
			current: pos,
			peak: 1 << height,
			peak_map,
			last_pos,
		}
	}
}

impl Iterator for Path {
	type Item = u64;

	fn next(&mut self) -> Option<Self::Item> {
		if self.current > self.last_pos {
			return None;
		}

		let next = Some(self.current);
		self.current += if (self.peak_map & self.peak) != 0 {
			1
		} else {
			2 * self.peak
		};
		self.peak <<= 1;
		next
	}
}

/// For a given starting position calculate the parent and sibling positions
/// for the branch/path from that position to the peak of the tree.
/// We will use the sibling positions to generate the "path" of a Merkle proof.
pub fn family_branch(pos: u64, last_pos: u64) -> Vec<(u64, u64)> {
	// loop going up the tree, from node to parent, as long as we stay inside
	// the tree (as defined by last_pos).
	let (peak_map, height) = peak_map_height(pos - 1);
	let mut peak = 1 << height;
	let mut branch = vec![];
	let mut current = pos;
	let mut sibling;
	while current < last_pos {
		if (peak_map & peak) != 0 {
			current += 1;
			sibling = current - 2 * peak;
		} else {
			current += 2 * peak;
			sibling = current - 1;
		};
		if current > last_pos {
			break;
		}
		branch.push((current, sibling));
		peak <<= 1;
	}
	branch
}

/// Gets the position of the rightmost node (i.e. leaf) beneath the provided subtree root.
pub fn bintree_rightmost(num: u64) -> u64 {
	num - bintree_postorder_height(num)
}

/// Gets the position of the rightmost node (i.e. leaf) beneath the provided subtree root.
pub fn bintree_leftmost(num: u64) -> u64 {
	let height = bintree_postorder_height(num);
	num + 2 - (2 << height)
}
