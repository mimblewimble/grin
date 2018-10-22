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

//! Database backed MMR.

use std::marker;

use core::hash::Hash;
use core::pmmr::{bintree_postorder_height, is_leaf, peak_map_height, peaks, HashOnlyBackend};
use ser::{PMMRIndexHashable, PMMRable};

/// Database backed MMR.
pub struct DBPMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + HashOnlyBackend,
{
	/// The last position in the PMMR
	last_pos: u64,
	/// The backend for this readonly PMMR
	backend: &'a mut B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> DBPMMR<'a, T, B>
where
	T: PMMRable + ::std::fmt::Debug,
	B: 'a + HashOnlyBackend,
{
	/// Build a new db backed MMR.
	pub fn new(backend: &'a mut B) -> DBPMMR<T, B> {
		DBPMMR {
			backend,
			last_pos: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new db backed MMR initialized to
	/// last_pos with the provided db backend.
	pub fn at(backend: &'a mut B, last_pos: u64) -> DBPMMR<T, B> {
		DBPMMR {
			backend,
			last_pos,
			_marker: marker::PhantomData,
		}
	}

	/// Get the unpruned size of the MMR.
	pub fn unpruned_size(&self) -> u64 {
		self.last_pos
	}

	/// Is the MMR empty?
	pub fn is_empty(&self) -> bool {
		self.last_pos == 0
	}

	/// Rewind the MMR to the specified position.
	pub fn rewind(&mut self, position: u64) -> Result<(), String> {
		// Identify which actual position we should rewind to as the provided
		// position is a leaf. We traverse the MMR to include any parent(s) that
		// need to be included for the MMR to be valid.
		let mut pos = position;
		while bintree_postorder_height(pos + 1) > 0 {
			pos += 1;
		}
		self.backend.rewind(pos)?;
		self.last_pos = pos;
		Ok(())
	}

	/// Get the hash element at provided position in the MMR.
	pub fn get_hash(&self, pos: u64) -> Option<Hash> {
		if pos > self.last_pos {
			// If we are beyond the rhs of the MMR return None.
			None
		} else if is_leaf(pos) {
			// If we are a leaf then get data from the backend.
			self.backend.get_hash(pos)
		} else {
			// If we are not a leaf then return None as only leaves have data.
			None
		}
	}

	/// Push a new element into the MMR. Computes new related peaks at
	/// the same time if applicable.
	pub fn push(&mut self, elmt: &T) -> Result<u64, String> {
		let elmt_pos = self.last_pos + 1;
		let mut current_hash = elmt.hash_with_index(elmt_pos - 1);

		let mut to_append = vec![current_hash];
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
				.get_hash(left_sibling)
				.ok_or("missing left sibling in tree, should not have been pruned")?;
			peak *= 2;
			pos += 1;
			current_hash = (left_hash, current_hash).hash_with_index(pos - 1);
			to_append.push(current_hash);
		}

		// append all the new nodes and update the MMR index
		self.backend.append(to_append)?;
		self.last_pos = pos;
		Ok(elmt_pos)
	}

	/// Return the vec of peak hashes for this MMR.
	pub fn peaks(&self) -> Vec<Hash> {
		let peaks_pos = peaks(self.last_pos);
		peaks_pos
			.into_iter()
			.filter_map(|pi| self.backend.get_hash(pi))
			.collect()
	}

	/// Return the overall root hash for this MMR.
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

	/// Validate all the hashes in the MMR.
	/// For every parent node we check hashes of the children produce the parent hash
	/// by hashing them together.
	pub fn validate(&self) -> Result<(), String> {
		// iterate on all parent nodes
		for n in 1..(self.last_pos + 1) {
			let height = bintree_postorder_height(n);
			if height > 0 {
				if let Some(hash) = self.get_hash(n) {
					let left_pos = n - (1 << height);
					let right_pos = n - 1;
					if let Some(left_child_hs) = self.get_hash(left_pos) {
						if let Some(right_child_hs) = self.get_hash(right_pos) {
							// hash the two child nodes together with parent_pos and compare
							if (left_child_hs, right_child_hs).hash_with_index(n - 1) != hash {
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
}
