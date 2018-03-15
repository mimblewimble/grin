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
use std::marker::PhantomData;
use core::hash::{Hash, Hashed};
use ser;
use ser::{Readable, Reader, Writeable, Writer};
use ser::{PMMRIndexHashable, PMMRable};
use util;
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

	/// Get a Hash/Element by insertion position. If include_data is true, will
	/// also return the associated data element
	fn get(&self, position: u64, include_data: bool) -> Option<(Hash, Option<T>)>;

	/// Get a Hash/Element by original insertion position (ignoring the remove
	/// list).
	fn get_from_file(&self, position: u64) -> Option<Hash>;

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

/// A Merkle proof.
/// Proves inclusion of an output (node) in the output MMR.
/// We can use this to prove an output was unspent at the time of a given block
/// as the root will match the output_root of the block header.
/// The path and left_right can be used to reconstruct the peak hash for a given tree
/// in the MMR.
/// The root is the result of hashing all the peaks together.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct MerkleProof {
	/// The root hash of the full Merkle tree (in an MMR the hash of all peaks)
	pub root: Hash,
	/// The hash of the element in the tree we care about
	pub node: Hash,
	/// The full list of peak hashes in the MMR
	pub peaks: Vec<Hash>,
	/// The siblings along the path of the tree as we traverse from node to peak
	pub path: Vec<Hash>,
	/// Order of siblings (left vs right) matters, so track this here for each
	/// path element
	pub left_right: Vec<bool>,
}

impl Writeable for MerkleProof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_fixed_bytes, &self.root],
			[write_fixed_bytes, &self.node],
			[write_u64, self.peaks.len() as u64],
			// note: path length used for both path and left_right vecs
			[write_u64, self.path.len() as u64]
		);

		try!(self.peaks.write(writer));
		try!(self.path.write(writer));

		// TODO - how to serialize/deserialize these boolean values as bytes?
		for x in &self.left_right {
			if *x {
				try!(writer.write_u8(1));
			} else {
				try!(writer.write_u8(0));
			}
		}

		Ok(())
	}
}

impl Readable for MerkleProof {
	fn read(reader: &mut Reader) -> Result<MerkleProof, ser::Error> {
		let root = Hash::read(reader)?;
		let node = Hash::read(reader)?;

		let (peaks_len, path_len) = ser_multiread!(reader, read_u64, read_u64);

		let mut peaks = Vec::with_capacity(peaks_len as usize);
		for _ in 0..peaks_len {
			peaks.push(Hash::read(reader)?);
		}
		let mut path = Vec::with_capacity(path_len as usize);
		for _ in 0..path_len {
			path.push(Hash::read(reader)?);
		}

		let left_right_bytes = reader.read_fixed_bytes(path_len as usize)?;
		let left_right = left_right_bytes.iter().map(|&x| x == 1).collect();
		Ok(MerkleProof {
			root,
			node,
			peaks,
			path,
			left_right,
		})
	}
}

impl Default for MerkleProof {
	fn default() -> MerkleProof {
		MerkleProof::empty()
	}
}

impl MerkleProof {
	/// The "empty" Merkle proof.
	/// Basically some reasonable defaults. Will not verify successfully.
	pub fn empty() -> MerkleProof {
		MerkleProof {
			root: Hash::zero(),
			node: Hash::zero(),
			peaks: vec![],
			path: vec![],
			left_right: vec![],
		}
	}

	/// Serialize the Merkle proof as a hex string (for api json endpoints)
	pub fn to_hex(&self) -> String {
		let mut vec = Vec::new();
		ser::serialize(&mut vec, &self).expect("serialization failed");
		util::to_hex(vec)
	}

	/// Convert hex string represenation back to a Merkle proof instance
	pub fn from_hex(hex: &str) -> Result<MerkleProof, String> {
		let bytes = util::from_hex(hex.to_string()).unwrap();
		let res = ser::deserialize(&mut &bytes[..])
			.map_err(|_| format!("failed to deserialize a Merkle Proof"))?;
		Ok(res)
	}

	/// Verify the Merkle proof.
	/// We do this by verifying the folloiwing -
	///  * inclusion of the node beneath a peak (via the Merkle path/branch of siblings)
	///  * inclusion of the peak in the "bag of peaks" beneath the root
	pub fn verify(&self) -> bool {
		// if we have no further elements in the path
		// then this proof verifies successfully if our node is
		// one of the peaks
		// and the peaks themselves hash to give the root
		if self.path.len() == 0 {
			if !self.peaks.contains(&self.node) {
				return false;
			}

			let mut bagged = None;
			for peak in self.peaks.iter().map(|&x| Some(x)) {
				bagged = match (bagged, peak) {
					(None, rhs) => rhs,
					(lhs, None) => lhs,
					(Some(lhs), Some(rhs)) => Some(lhs.hash_with(rhs)),
				}
			}
			return bagged == Some(self.root);
		}

		let mut path = self.path.clone();
		let sibling = path.remove(0);
		let mut left_right = self.left_right.clone();

		// hash our node and sibling together (noting left/right position of the
		// sibling)
		let parent = if left_right.remove(0) {
			self.node.hash_with(sibling)
		} else {
			sibling.hash_with(self.node)
		};

		let proof = MerkleProof {
			root: self.root,
			node: parent,
			peaks: self.peaks.clone(),
			path,
			left_right,
		};

		proof.verify()
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
	T: PMMRable,
	B: 'a + Backend<T>,
{
	last_pos: u64,
	backend: &'a mut B,
	// only needed for parameterizing Backend
	writeable: PhantomData<T>,
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
			writeable: PhantomData,
		}
	}

	/// Build a new prunable Merkle Mountain Range pre-initialized until
	/// last_pos
	/// with the provided backend.
	pub fn at(backend: &'a mut B, last_pos: u64) -> PMMR<T, B> {
		PMMR {
			last_pos: last_pos,
			backend: backend,
			writeable: PhantomData,
		}
	}

	/// Computes the root of the MMR. Find all the peaks in the current
	/// tree and "bags" them to get a single peak.
	pub fn root(&self) -> Hash {
		let peaks_pos = peaks(self.last_pos);
		let peaks: Vec<Option<Hash>> = peaks_pos
			.into_iter()
			.map(|pi| {
				// here we want to get from underlying hash file
				// as the pos *may* have been "removed"
				self.backend.get_from_file(pi)
			})
			.collect();

		let mut ret = None;
		for peak in peaks {
			ret = match (ret, peak) {
				(None, x) => x,
				(Some(hash), None) => Some(hash),
				(Some(lhash), Some(rhash)) => Some(lhash.hash_with(rhash)),
			}
		}
		ret.expect("no root, invalid tree")
	}

	/// Build a Merkle proof for the element at the given position in the MMR
	pub fn merkle_proof(&self, pos: u64) -> Result<MerkleProof, String> {
		debug!(
			LOGGER,
			"merkle_proof (via rewind) - {}, last_pos {}", pos, self.last_pos
		);

		if !is_leaf(pos) {
			return Err(format!("not a leaf at pos {}", pos));
		}

		let root = self.root();

		let node = self.get(pos, false)
			.ok_or(format!("no element at pos {}", pos))?
			.0;

		let family_branch = family_branch(pos, self.last_pos);
		let left_right = family_branch.iter().map(|x| x.2).collect::<Vec<_>>();

		let path = family_branch
			.iter()
			.filter_map(|x| {
				// we want to find siblings here even if they
				// have been "removed" from the MMR
				self.get_from_file(x.1)
			})
			.collect::<Vec<_>>();

		let peaks = peaks(self.last_pos)
			.iter()
			.filter_map(|&x| {
				let res = self.get_from_file(x);
				res
			})
			.collect::<Vec<_>>();

		let proof = MerkleProof {
			root,
			node,
			path,
			peaks,
			left_right,
		};

		Ok(proof)
	}

	/// Push a new element into the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push(&mut self, elmt: T) -> Result<u64, String> {
		let elmt_pos = self.last_pos + 1;
		let mut current_hash = elmt.hash_with_index(elmt_pos);

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

			current_hash = left_hash + current_hash;

			to_append.push((current_hash.clone(), None));
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
		if let None = self.backend.get(position, false) {
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
			let (parent, sibling, _) = family(current);

			to_prune.push(current);

			if parent > self.last_pos {
				// can't prune when our parent isn't here yet
				break;
			}

			// if we have a pruned sibling, we can continue up the tree
			// otherwise we're done
			if let None = self.backend.get(sibling, false) {
				current = parent;
			} else {
				break;
			}
		}

		self.backend.remove(to_prune, index)?;
		Ok(true)
	}

	/// Helper function to get a node at a given position from
	/// the backend.
	pub fn get(&self, position: u64, include_data: bool) -> Option<(Hash, Option<T>)> {
		if position > self.last_pos {
			None
		} else {
			self.backend.get(position, include_data)
		}
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		if position > self.last_pos {
			None
		} else {
			self.backend.get_from_file(position)
		}
	}

	/// Helper function to get the last N nodes inserted, i.e. the last
	/// n nodes along the bottom of the tree
	pub fn get_last_n_insertions(&self, n: u64) -> Vec<(Hash, Option<T>)> {
		let mut return_vec = Vec::new();
		let mut last_leaf = self.last_pos;
		let size = self.unpruned_size();
		// Special case that causes issues in bintree functions,
		// just return
		if size == 1 {
			return_vec.push(self.backend.get(last_leaf, true).unwrap());
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
			return_vec.push(self.backend.get(last_leaf, true).unwrap());

			last_leaf = bintree_jump_left_sibling(last_leaf);
		}
		return_vec
	}

	/// Walks all unpruned nodes in the MMR and revalidate all parent hashes
	pub fn validate(&self) -> Result<(), String> {
		// iterate on all parent nodes
		for n in 1..(self.last_pos + 1) {
			if bintree_postorder_height(n) > 0 {
				if let Some(hs) = self.get(n, false) {
					// take the left and right children, if they exist
					let left_pos = bintree_move_down_left(n).ok_or(format!("left_pos not found"))?;
					let right_pos = bintree_jump_right_sibling(left_pos);

					if let Some(left_child_hs) = self.get(left_pos, false) {
						if let Some(right_child_hs) = self.get(right_pos, false) {
							// add hashes and compare
							if left_child_hs.0 + right_child_hs.0 != hs.0 {
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
				let ohs = self.get(m + 1, false);
				match ohs {
					Some(hs) => hashes.push_str(&format!("{} ", hs.0)),
					None => hashes.push_str(&format!("{:>8} ", "??")),
				}
			}
			trace!(LOGGER, "{}", idx);
			trace!(LOGGER, "{}", hashes);
		}
	}

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
			let (parent, sibling, _) = family(current);

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
						let (parent, _, _) = family(cursor);
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
	while bintree_postorder_height(sz + 1) > 0 {
		sz += 1;
	}
	peaks(sz)
		.iter()
		.map(|n| (1 << bintree_postorder_height(*n)) as u64)
		.sum()
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
	bintree_postorder_height(pos) == 0
}

/// Calculates the positions of the parent and sibling of the node at the
/// provided position. Also returns a boolean representing whether the sibling is on left
/// branch or right branch (left=0, right=1)
pub fn family(pos: u64) -> (u64, u64, bool) {
	let pos_height = bintree_postorder_height(pos);
	let next_height = bintree_postorder_height(pos + 1);
	if next_height > pos_height {
		let sibling = bintree_jump_left_sibling(pos);
		let parent = pos + 1;
		(parent, sibling, false)
	} else {
		let sibling = bintree_jump_right_sibling(pos);
		let parent = sibling + 1;
		(parent, sibling, true)
	}
}

/// For a given starting position calculate the parent and sibling positions
/// for the branch/path from that position to the peak of the tree.
/// We will use the sibling positions to generate the "path" of a Merkle proof.
pub fn family_branch(pos: u64, last_pos: u64) -> Vec<(u64, u64, bool)> {
	// loop going up the tree, from node to parent, as long as we stay inside
	// the tree (as defined by last_pos).
	let mut branch = vec![];
	let mut current = pos;
	while current + 1 <= last_pos {
		let (parent, sibling, sibling_branch) = family(current);
		if parent > last_pos {
			break;
		}
		branch.push((parent, sibling, sibling_branch));

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
	use ser::{Error, Readable, Writeable};
	use core::{Reader, Writer};
	use core::hash::Hash;
	use ser::{PMMRIndexHashable, PMMRable};

	/// Simple MMR backend implementation based on a Vector. Pruning does not
	/// compact the Vec itself.
	#[derive(Clone, Debug)]
	pub struct VecBackend<T>
	where
		T: PMMRable,
	{
		/// Backend elements
		pub elems: Vec<Option<(Hash, Option<T>)>>,
		/// Positions of removed elements
		pub remove_list: Vec<u64>,
	}

	impl<T> Backend<T> for VecBackend<T>
	where
		T: PMMRable,
	{
		fn append(&mut self, _position: u64, data: Vec<(Hash, Option<T>)>) -> Result<(), String> {
			self.elems.append(&mut map_vec!(data, |d| Some(d.clone())));
			Ok(())
		}

		fn get(&self, position: u64, _include_data: bool) -> Option<(Hash, Option<T>)> {
			if self.remove_list.contains(&position) {
				None
			} else {
				self.elems[(position - 1) as usize].clone()
			}
		}

		fn get_from_file(&self, position: u64) -> Option<Hash> {
			if let Some(ref x) = self.elems[(position - 1) as usize] {
				Some(x.0)
			} else {
				None
			}
		}

		fn remove(&mut self, positions: Vec<u64>, _index: u32) -> Result<(), String> {
			for n in positions {
				self.remove_list.push(n)
			}
			Ok(())
		}

		fn rewind(&mut self, position: u64, _index: u32) -> Result<(), String> {
			self.elems = self.elems[0..(position as usize) + 1].to_vec();
			Ok(())
		}

		fn get_data_file_path(&self) -> String {
			"".to_string()
		}

		fn dump_stats(&self) {}
	}

	impl<T> VecBackend<T>
	where
		T: PMMRable,
	{
		/// Instantiates a new VecBackend<T>
		pub fn new() -> VecBackend<T> {
			VecBackend {
				elems: vec![],
				remove_list: vec![],
			}
		}

		/// Current number of elements in the underlying Vec.
		pub fn used_size(&self) -> usize {
			let mut usz = self.elems.len();
			for (idx, _) in self.elems.iter().enumerate() {
				let idx = idx as u64;
				if self.remove_list.contains(&idx) {
					usz -= 1;
				}
			}
			usz
		}
	}

	#[test]
	fn test_leaf_index() {
		assert_eq!(n_leaves(1), 1);
		assert_eq!(n_leaves(2), 2);
		assert_eq!(n_leaves(4), 3);
		assert_eq!(n_leaves(5), 4);
		assert_eq!(n_leaves(8), 5);
		assert_eq!(n_leaves(9), 6);
	}

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
	fn first_100_mmr_heights() {
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

	// Trst our n_leaves impl does the right thing for various MMR sizes
	#[test]
	fn various_n_leaves() {
		assert_eq!(n_leaves(1), 1);
		// 2 is not a valid size for a tree, but n_leaves rounds up to next valid tree
		// size
		assert_eq!(n_leaves(2), 2);
		assert_eq!(n_leaves(3), 2);
		assert_eq!(n_leaves(7), 4);
	}

	/// Find parent and sibling positions for various node positions.
	#[test]
	fn various_families() {
		// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3
		assert_eq!(family(1), (3, 2, true));
		assert_eq!(family(2), (3, 1, false));
		assert_eq!(family(3), (7, 6, true));
		assert_eq!(family(4), (6, 5, true));
		assert_eq!(family(5), (6, 4, false));
		assert_eq!(family(6), (7, 3, false));
		assert_eq!(family(7), (15, 14, true));
		assert_eq!(family(1_000), (1_001, 997, false));
	}

	#[test]
	fn various_branches() {
		// the two leaf nodes in a 3 node tree (height 1)
		assert_eq!(family_branch(1, 3), [(3, 2, true)]);
		assert_eq!(family_branch(2, 3), [(3, 1, false)]);

		// the root node in a 3 node tree
		assert_eq!(family_branch(3, 3), []);

		// leaf node in a larger tree of 7 nodes (height 2)
		assert_eq!(family_branch(1, 7), [(3, 2, true), (7, 6, true)]);

		// note these only go as far up as the local peak, not necessarily the single
		// root
		assert_eq!(family_branch(1, 4), [(3, 2, true)]);
		// pos 4 in a tree of size 4 is a local peak
		assert_eq!(family_branch(4, 4), []);
		// pos 4 in a tree of size 5 is also still a local peak
		assert_eq!(family_branch(4, 5), []);
		// pos 4 in a tree of size 6 has a parent and a sibling
		assert_eq!(family_branch(4, 6), [(6, 5, true)]);
		// a tree of size 7 is all under a single root
		assert_eq!(family_branch(4, 7), [(6, 5, true), (7, 3, false)]);

		// ok now for a more realistic one, a tree with over a million nodes in it
		// find the "family path" back up the tree from a leaf node at 0
		// Note: the first two entries in the branch are consistent with a small 7 node
		// tree Note: each sibling is on the left branch, this is an example of the
		// largest possible list of peaks before we start combining them into larger
		// peaks.
		assert_eq!(
			family_branch(1, 1_049_000),
			[
				(3, 2, true),
				(7, 6, true),
				(15, 14, true),
				(31, 30, true),
				(63, 62, true),
				(127, 126, true),
				(255, 254, true),
				(511, 510, true),
				(1023, 1022, true),
				(2047, 2046, true),
				(4095, 4094, true),
				(8191, 8190, true),
				(16383, 16382, true),
				(32767, 32766, true),
				(65535, 65534, true),
				(131071, 131070, true),
				(262143, 262142, true),
				(524287, 524286, true),
				(1048575, 1048574, true),
			]
		);
	}

	#[test]
	fn some_peaks() {
		// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3
		let empty: Vec<u64> = vec![];
		assert_eq!(peaks(1), [1]);
		assert_eq!(peaks(2), empty);
		assert_eq!(peaks(3), [3]);
		assert_eq!(peaks(4), [3, 4]);
		assert_eq!(peaks(5), empty);
		assert_eq!(peaks(6), empty);
		assert_eq!(peaks(7), [7]);
		assert_eq!(peaks(8), [7, 8]);
		assert_eq!(peaks(9), empty);
		assert_eq!(peaks(10), [7, 10]);
		assert_eq!(peaks(11), [7, 10, 11]);
		assert_eq!(peaks(22), [15, 22]);
		assert_eq!(peaks(32), [31, 32]);
		assert_eq!(peaks(35), [31, 34, 35]);
		assert_eq!(peaks(42), [31, 38, 41, 42]);

		// large realistic example with almost 1.5 million nodes
		// note the distance between peaks decreases toward the right (trees get
		// smaller)
		assert_eq!(
			peaks(1048555),
			[
				524287, 786430, 917501, 983036, 1015803, 1032186, 1040377, 1044472, 1046519,
				1047542, 1048053, 1048308, 1048435, 1048498, 1048529, 1048544, 1048551, 1048554,
				1048555,
			],
		);
	}

	#[derive(Copy, Clone, Debug, PartialEq, Eq)]
	struct TestElem([u32; 4]);

	impl PMMRable for TestElem {
		fn len() -> usize {
			16
		}
	}

	impl Writeable for TestElem {
		fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
			try!(writer.write_u32(self.0[0]));
			try!(writer.write_u32(self.0[1]));
			try!(writer.write_u32(self.0[2]));
			writer.write_u32(self.0[3])
		}
	}

	impl Readable for TestElem {
		fn read(reader: &mut Reader) -> Result<TestElem, Error> {
			Ok(TestElem([
				reader.read_u32()?,
				reader.read_u32()?,
				reader.read_u32()?,
				reader.read_u32()?,
			]))
		}
	}

	#[test]
	fn empty_merkle_proof() {
		let proof = MerkleProof::empty();
		assert_eq!(proof.verify(), false);
	}

	#[test]
	fn pmmr_merkle_proof() {
		// 0 0 1 0 0 1 2 0 0 1 0 0 1 2 3

		let mut ba = VecBackend::new();
		let mut pmmr = PMMR::new(&mut ba);

		pmmr.push(TestElem([0, 0, 0, 1])).unwrap();
		assert_eq!(pmmr.last_pos, 1);
		let proof = pmmr.merkle_proof(1).unwrap();
		let root = pmmr.root();
		assert_eq!(proof.peaks, [root]);
		assert!(proof.path.is_empty());
		assert!(proof.left_right.is_empty());
		assert!(proof.verify());

		// push two more elements into the PMMR
		pmmr.push(TestElem([0, 0, 0, 2])).unwrap();
		pmmr.push(TestElem([0, 0, 0, 3])).unwrap();
		assert_eq!(pmmr.last_pos, 4);

		let proof1 = pmmr.merkle_proof(1).unwrap();
		assert_eq!(proof1.peaks.len(), 2);
		assert_eq!(proof1.path.len(), 1);
		assert_eq!(proof1.left_right, [true]);
		assert!(proof1.verify());

		let proof2 = pmmr.merkle_proof(2).unwrap();
		assert_eq!(proof2.peaks.len(), 2);
		assert_eq!(proof2.path.len(), 1);
		assert_eq!(proof2.left_right, [false]);
		assert!(proof2.verify());

		// check that we cannot generate a merkle proof for pos 3 (not a leaf node)
		assert_eq!(
			pmmr.merkle_proof(3).err(),
			Some(format!("not a leaf at pos 3"))
		);

		let proof4 = pmmr.merkle_proof(4).unwrap();
		assert_eq!(proof4.peaks.len(), 2);
		assert!(proof4.path.is_empty());
		assert!(proof4.left_right.is_empty());
		assert!(proof4.verify());

		// now add a few more elements to the PMMR to build a larger merkle proof
		for x in 4..1000 {
			pmmr.push(TestElem([0, 0, 0, x])).unwrap();
		}
		let proof = pmmr.merkle_proof(1).unwrap();
		assert_eq!(proof.peaks.len(), 8);
		assert_eq!(proof.path.len(), 9);
		assert_eq!(proof.left_right.len(), 9);
		assert!(proof.verify());
	}

	#[test]
	fn pmmr_merkle_proof_prune_and_rewind() {
		let mut ba = VecBackend::new();
		let mut pmmr = PMMR::new(&mut ba);
		pmmr.push(TestElem([0, 0, 0, 1])).unwrap();
		pmmr.push(TestElem([0, 0, 0, 2])).unwrap();
		let proof = pmmr.merkle_proof(2).unwrap();

		// now prune an element and check we can still generate
		// the correct Merkle proof for the other element (after sibling pruned)
		pmmr.prune(1, 1).unwrap();
		let proof_2 = pmmr.merkle_proof(2).unwrap();
		assert_eq!(proof, proof_2);
	}

	#[test]
	fn merkle_proof_ser_deser() {
		let mut ba = VecBackend::new();
		let mut pmmr = PMMR::new(&mut ba);
		for x in 0..15 {
			pmmr.push(TestElem([0, 0, 0, x])).unwrap();
		}
		let proof = pmmr.merkle_proof(9).unwrap();
		assert!(proof.verify());

		let mut vec = Vec::new();
		ser::serialize(&mut vec, &proof).expect("serialization failed");
		let proof_2: MerkleProof = ser::deserialize(&mut &vec[..]).unwrap();

		assert_eq!(proof, proof_2);
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
		pmmr.push(elems[0]).unwrap();
		let node_hash = elems[0].hash_with_index(1);
		assert_eq!(pmmr.root(), node_hash);
		assert_eq!(pmmr.unpruned_size(), 1);
		pmmr.dump(false);

		// two elements
		pmmr.push(elems[1]).unwrap();
		let sum2 = elems[0].hash_with_index(1) + elems[1].hash_with_index(2);
		pmmr.dump(false);
		assert_eq!(pmmr.root(), sum2);
		assert_eq!(pmmr.unpruned_size(), 3);

		// three elements
		pmmr.push(elems[2]).unwrap();
		let sum3 = sum2 + elems[2].hash_with_index(4);
		pmmr.dump(false);
		assert_eq!(pmmr.root(), sum3);
		assert_eq!(pmmr.unpruned_size(), 4);

		// four elements
		pmmr.push(elems[3]).unwrap();
		let sum_one = elems[2].hash_with_index(4) + elems[3].hash_with_index(5);
		let sum4 = sum2 + sum_one;
		pmmr.dump(false);
		assert_eq!(pmmr.root(), sum4);
		assert_eq!(pmmr.unpruned_size(), 7);

		// five elements
		pmmr.push(elems[4]).unwrap();
		let sum3 = sum4 + elems[4].hash_with_index(8);
		pmmr.dump(false);
		assert_eq!(pmmr.root(), sum3);
		assert_eq!(pmmr.unpruned_size(), 8);

		// six elements
		pmmr.push(elems[5]).unwrap();
		let sum6 = sum4 + (elems[4].hash_with_index(8) + elems[5].hash_with_index(9));
		assert_eq!(pmmr.root(), sum6.clone());
		assert_eq!(pmmr.unpruned_size(), 10);

		// seven elements
		pmmr.push(elems[6]).unwrap();
		let sum7 = sum6 + elems[6].hash_with_index(11);
		assert_eq!(pmmr.root(), sum7);
		assert_eq!(pmmr.unpruned_size(), 11);

		// eight elements
		pmmr.push(elems[7]).unwrap();
		let sum8 = sum4
			+ ((elems[4].hash_with_index(8) + elems[5].hash_with_index(9))
				+ (elems[6].hash_with_index(11) + elems[7].hash_with_index(12)));
		assert_eq!(pmmr.root(), sum8);
		assert_eq!(pmmr.unpruned_size(), 15);

		// nine elements
		pmmr.push(elems[8]).unwrap();
		let sum9 = sum8 + elems[8].hash_with_index(16);
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
			TestElem([1, 0, 0, 0]),
		];

		let mut ba = VecBackend::new();
		let mut pmmr = PMMR::new(&mut ba);

		// test when empty
		let res = pmmr.get_last_n_insertions(19);
		assert!(res.len() == 0);

		pmmr.push(elems[0]).unwrap();
		let res = pmmr.get_last_n_insertions(19);
		assert!(res.len() == 1);

		pmmr.push(elems[1]).unwrap();

		let res = pmmr.get_last_n_insertions(12);
		assert!(res.len() == 2);

		pmmr.push(elems[2]).unwrap();

		let res = pmmr.get_last_n_insertions(2);
		assert!(res.len() == 2);

		pmmr.push(elems[3]).unwrap();

		let res = pmmr.get_last_n_insertions(19);
		assert!(res.len() == 4);

		pmmr.push(elems[5]).unwrap();
		pmmr.push(elems[6]).unwrap();
		pmmr.push(elems[7]).unwrap();
		pmmr.push(elems[8]).unwrap();

		let res = pmmr.get_last_n_insertions(7);
		assert!(res.len() == 7);
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

		let orig_root: Hash;
		let sz: u64;
		let mut ba = VecBackend::new();
		{
			let mut pmmr = PMMR::new(&mut ba);
			for elem in &elems[..] {
				pmmr.push(*elem).unwrap();
			}
			orig_root = pmmr.root();
			sz = pmmr.unpruned_size();
		}

		// pruning a leaf with no parent should do nothing
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			pmmr.prune(16, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 16);

		// pruning leaves with no shared parent just removes 1 element
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			pmmr.prune(2, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 15);

		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			pmmr.prune(4, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 14);

		// pruning a non-leaf node has no effect
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			pmmr.prune(3, 0).unwrap_err();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 14);

		// pruning sibling removes subtree
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			pmmr.prune(5, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 12);

		// pruning all leaves under level >1 removes all subtree
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			pmmr.prune(1, 0).unwrap();
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 9);

		// pruning everything should only leave us with a single peak
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut ba, sz);
			for n in 1..16 {
				let _ = pmmr.prune(n, 0);
			}
			assert_eq!(orig_root, pmmr.root());
		}
		assert_eq!(ba.used_size(), 1);
	}

	#[test]
	fn pmmr_next_pruned_idx() {
		let mut pl = PruneList::new();

		assert_eq!(pl.pruned_nodes.len(), 0);
		assert_eq!(pl.next_pruned_idx(1), Some(0));
		assert_eq!(pl.next_pruned_idx(2), Some(0));
		assert_eq!(pl.next_pruned_idx(3), Some(0));

		pl.add(2);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes, [2]);
		assert_eq!(pl.next_pruned_idx(1), Some(0));
		assert_eq!(pl.next_pruned_idx(2), None);
		assert_eq!(pl.next_pruned_idx(3), Some(1));
		assert_eq!(pl.next_pruned_idx(4), Some(1));

		pl.add(1);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes, [3]);
		assert_eq!(pl.next_pruned_idx(1), None);
		assert_eq!(pl.next_pruned_idx(2), None);
		assert_eq!(pl.next_pruned_idx(3), None);
		assert_eq!(pl.next_pruned_idx(4), Some(1));
		assert_eq!(pl.next_pruned_idx(5), Some(1));

		pl.add(3);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes, [3]);
		assert_eq!(pl.next_pruned_idx(1), None);
		assert_eq!(pl.next_pruned_idx(2), None);
		assert_eq!(pl.next_pruned_idx(3), None);
		assert_eq!(pl.next_pruned_idx(4), Some(1));
		assert_eq!(pl.next_pruned_idx(5), Some(1));
	}

	#[test]
	fn pmmr_prune_leaf_shift() {
		let mut pl = PruneList::new();

		// start with an empty prune list (nothing shifted)
		assert_eq!(pl.pruned_nodes.len(), 0);
		assert_eq!(pl.get_leaf_shift(1), Some(0));
		assert_eq!(pl.get_leaf_shift(2), Some(0));
		assert_eq!(pl.get_leaf_shift(4), Some(0));

		// now add a single leaf pos to the prune list
		// note this does not shift anything (we only start shifting after pruning a
		// parent)
		pl.add(1);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes, [1]);
		assert_eq!(pl.get_leaf_shift(1), Some(0));
		assert_eq!(pl.get_leaf_shift(2), Some(0));
		assert_eq!(pl.get_leaf_shift(3), Some(0));
		assert_eq!(pl.get_leaf_shift(4), Some(0));

		// now add the sibling leaf pos (pos 1 and pos 2) which will prune the parent
		// at pos 3 this in turn will "leaf shift" the leaf at pos 3 by 2
		pl.add(2);
		assert_eq!(pl.pruned_nodes.len(), 1);
		assert_eq!(pl.pruned_nodes, [3]);
		assert_eq!(pl.get_leaf_shift(1), None);
		assert_eq!(pl.get_leaf_shift(2), None);
		assert_eq!(pl.get_leaf_shift(3), Some(2));
		assert_eq!(pl.get_leaf_shift(4), Some(2));
		assert_eq!(pl.get_leaf_shift(5), Some(2));

		// now prune an additional leaf at pos 4
		// leaf offset of subsequent pos will be 2
		// 00100120
		pl.add(4);
		assert_eq!(pl.pruned_nodes, [3, 4]);
		assert_eq!(pl.get_leaf_shift(1), None);
		assert_eq!(pl.get_leaf_shift(2), None);
		assert_eq!(pl.get_leaf_shift(3), Some(2));
		assert_eq!(pl.get_leaf_shift(4), Some(2));
		assert_eq!(pl.get_leaf_shift(5), Some(2));
		assert_eq!(pl.get_leaf_shift(6), Some(2));
		assert_eq!(pl.get_leaf_shift(7), Some(2));
		assert_eq!(pl.get_leaf_shift(8), Some(2));

		// now prune the sibling at pos 5
		// the two smaller subtrees (pos 3 and pos 6) are rolled up to larger subtree
		// (pos 7) the leaf offset is now 4 to cover entire subtree containing first
		// 4 leaves 00100120
		pl.add(5);
		assert_eq!(pl.pruned_nodes, [7]);
		assert_eq!(pl.get_leaf_shift(1), None);
		assert_eq!(pl.get_leaf_shift(2), None);
		assert_eq!(pl.get_leaf_shift(3), None);
		assert_eq!(pl.get_leaf_shift(4), None);
		assert_eq!(pl.get_leaf_shift(5), None);
		assert_eq!(pl.get_leaf_shift(6), None);
		assert_eq!(pl.get_leaf_shift(7), Some(4));
		assert_eq!(pl.get_leaf_shift(8), Some(4));
		assert_eq!(pl.get_leaf_shift(9), Some(4));

		// now check we can prune some of these in an arbitrary order
		// final result is one leaf (pos 2) and one small subtree (pos 6) pruned
		// with leaf offset of 2 to account for the pruned subtree
		let mut pl = PruneList::new();
		pl.add(2);
		pl.add(5);
		pl.add(4);
		assert_eq!(pl.pruned_nodes, [2, 6]);
		assert_eq!(pl.get_leaf_shift(1), Some(0));
		assert_eq!(pl.get_leaf_shift(2), Some(0));
		assert_eq!(pl.get_leaf_shift(3), Some(0));
		assert_eq!(pl.get_leaf_shift(4), None);
		assert_eq!(pl.get_leaf_shift(5), None);
		assert_eq!(pl.get_leaf_shift(6), Some(2));
		assert_eq!(pl.get_leaf_shift(7), Some(2));
		assert_eq!(pl.get_leaf_shift(8), Some(2));
		assert_eq!(pl.get_leaf_shift(9), Some(2));

		pl.add(1);
		assert_eq!(pl.pruned_nodes, [7]);
		assert_eq!(pl.get_leaf_shift(1), None);
		assert_eq!(pl.get_leaf_shift(2), None);
		assert_eq!(pl.get_leaf_shift(3), None);
		assert_eq!(pl.get_leaf_shift(4), None);
		assert_eq!(pl.get_leaf_shift(5), None);
		assert_eq!(pl.get_leaf_shift(6), None);
		assert_eq!(pl.get_leaf_shift(7), Some(4));
		assert_eq!(pl.get_leaf_shift(8), Some(4));
		assert_eq!(pl.get_leaf_shift(9), Some(4));
	}

	#[test]
	fn pmmr_prune_shift() {
		let mut pl = PruneList::new();
		assert!(pl.pruned_nodes.is_empty());
		assert_eq!(pl.get_shift(1), Some(0));
		assert_eq!(pl.get_shift(2), Some(0));
		assert_eq!(pl.get_shift(3), Some(0));

		// prune a single leaf node
		// pruning only a leaf node does not shift any subsequent pos
		// we will only start shifting when a parent can be pruned
		pl.add(1);
		assert_eq!(pl.pruned_nodes, [1]);
		assert_eq!(pl.get_shift(1), Some(0));
		assert_eq!(pl.get_shift(2), Some(0));
		assert_eq!(pl.get_shift(3), Some(0));

		pl.add(2);
		assert_eq!(pl.pruned_nodes, [3]);
		assert_eq!(pl.get_shift(1), None);
		assert_eq!(pl.get_shift(2), None);
		// pos 3 is in the prune list, so removed but not compacted, but still shifted
		assert_eq!(pl.get_shift(3), Some(2));
		assert_eq!(pl.get_shift(4), Some(2));
		assert_eq!(pl.get_shift(5), Some(2));
		assert_eq!(pl.get_shift(6), Some(2));

		// pos 3 is not a leaf and is already in prune list
		// prune it and check we are still consistent
		pl.add(3);
		assert_eq!(pl.pruned_nodes, [3]);
		assert_eq!(pl.get_shift(1), None);
		assert_eq!(pl.get_shift(2), None);
		// pos 3 is in the prune list, so removed but not compacted, but still shifted
		assert_eq!(pl.get_shift(3), Some(2));
		assert_eq!(pl.get_shift(4), Some(2));
		assert_eq!(pl.get_shift(5), Some(2));
		assert_eq!(pl.get_shift(6), Some(2));

		pl.add(4);
		assert_eq!(pl.pruned_nodes, [3, 4]);
		assert_eq!(pl.get_shift(1), None);
		assert_eq!(pl.get_shift(2), None);
		// pos 3 is in the prune list, so removed but not compacted, but still shifted
		assert_eq!(pl.get_shift(3), Some(2));
		// pos 4 is also in the prune list and also shifted by same amount
		assert_eq!(pl.get_shift(4), Some(2));
		// subsequent nodes also shifted consistently
		assert_eq!(pl.get_shift(5), Some(2));
		assert_eq!(pl.get_shift(6), Some(2));

		pl.add(5);
		assert_eq!(pl.pruned_nodes, [7]);
		assert_eq!(pl.get_shift(1), None);
		assert_eq!(pl.get_shift(2), None);
		assert_eq!(pl.get_shift(3), None);
		assert_eq!(pl.get_shift(4), None);
		assert_eq!(pl.get_shift(5), None);
		assert_eq!(pl.get_shift(6), None);
		// everything prior to pos 7 is compacted away
		// pos 7 is shifted by 6 to account for this
		assert_eq!(pl.get_shift(7), Some(6));
		assert_eq!(pl.get_shift(8), Some(6));
		assert_eq!(pl.get_shift(9), Some(6));

		// prune a bunch more
		for x in 6..1000 {
			pl.add(x);
		}
		// and check we shift by a large number (hopefully the correct number...)
		assert_eq!(pl.get_shift(1010), Some(996));

		let mut pl = PruneList::new();
		pl.add(2);
		pl.add(5);
		pl.add(4);
		assert_eq!(pl.pruned_nodes, [2, 6]);
		assert_eq!(pl.get_shift(1), Some(0));
		assert_eq!(pl.get_shift(2), Some(0));
		assert_eq!(pl.get_shift(3), Some(0));
		assert_eq!(pl.get_shift(4), None);
		assert_eq!(pl.get_shift(5), None);
		assert_eq!(pl.get_shift(6), Some(2));
		assert_eq!(pl.get_shift(7), Some(2));
		assert_eq!(pl.get_shift(8), Some(2));
		assert_eq!(pl.get_shift(9), Some(2));

		// TODO - put some of these tests back in place for completeness

		//
		// let mut pl = PruneList::new();
		// pl.add(4);
		// assert_eq!(pl.pruned_nodes.len(), 1);
		// assert_eq!(pl.pruned_nodes, [4]);
		// assert_eq!(pl.get_shift(1), Some(0));
		// assert_eq!(pl.get_shift(2), Some(0));
		// assert_eq!(pl.get_shift(3), Some(0));
		// assert_eq!(pl.get_shift(4), None);
		// assert_eq!(pl.get_shift(5), Some(1));
		// assert_eq!(pl.get_shift(6), Some(1));
		//
		//
		// pl.add(5);
		// assert_eq!(pl.pruned_nodes.len(), 1);
		// assert_eq!(pl.pruned_nodes[0], 6);
		// assert_eq!(pl.get_shift(8), Some(3));
		// assert_eq!(pl.get_shift(2), Some(0));
		// assert_eq!(pl.get_shift(5), None);
		//
		// pl.add(2);
		// assert_eq!(pl.pruned_nodes.len(), 2);
		// assert_eq!(pl.pruned_nodes[0], 2);
		// assert_eq!(pl.get_shift(8), Some(4));
		// assert_eq!(pl.get_shift(1), Some(0));
		//
		// pl.add(8);
		// pl.add(11);
		// assert_eq!(pl.pruned_nodes.len(), 4);
		//
		// pl.add(1);
		// assert_eq!(pl.pruned_nodes.len(), 3);
		// assert_eq!(pl.pruned_nodes[0], 7);
		// assert_eq!(pl.get_shift(12), Some(9));
		//
		// pl.add(12);
		// assert_eq!(pl.pruned_nodes.len(), 3);
		// assert_eq!(pl.get_shift(12), None);
		// assert_eq!(pl.get_shift(9), Some(8));
		// assert_eq!(pl.get_shift(17), Some(11));
	}

	#[test]
	fn n_size_check() {
		assert_eq!(n_leaves(1), 1);
		assert_eq!(n_leaves(2), 2);
		assert_eq!(n_leaves(3), 2);
		assert_eq!(n_leaves(4), 3);
		assert_eq!(n_leaves(5), 4);
		assert_eq!(n_leaves(7), 4);
		assert_eq!(n_leaves(8), 5);
		assert_eq!(n_leaves(9), 6);
		assert_eq!(n_leaves(10), 6);
	}
}
