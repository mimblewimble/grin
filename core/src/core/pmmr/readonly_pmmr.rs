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

//! Readonly view of a PMMR.

use std::marker;

use core::hash::{Hash, ZERO_HASH};
use core::pmmr::pmmr::{bintree_rightmost, insertion_to_pmmr_index, peaks};
use core::pmmr::{is_leaf, Backend};
use ser::{PMMRIndexHashable, PMMRable};

/// Readonly view of a PMMR.
pub struct ReadonlyPMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	/// The last position in the PMMR
	last_pos: u64,
	/// The backend for this readonly PMMR
	backend: &'a B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> ReadonlyPMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	/// Build a new readonly PMMR.
	pub fn new(backend: &'a B) -> ReadonlyPMMR<T, B> {
		ReadonlyPMMR {
			backend,
			last_pos: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new readonly PMMR pre-initialized to
	/// last_pos with the provided backend.
	pub fn at(backend: &'a B, last_pos: u64) -> ReadonlyPMMR<T, B> {
		ReadonlyPMMR {
			backend,
			last_pos,
			_marker: marker::PhantomData,
		}
	}

	/// Get the data element at provided position in the MMR.
	pub fn get_data(&self, pos: u64) -> Option<T::E> {
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

	/// Get the hash at provided position in the MMR.
	pub fn get_hash(&self, pos: u64) -> Option<Hash> {
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

	/// Is the MMR empty?
	pub fn is_empty(&self) -> bool {
		self.last_pos == 0
	}

	/// Computes the root of the MMR. Find all the peaks in the current
	/// tree and "bags" them to get a single peak.
	pub fn root(&self) -> Hash {
		if self.is_empty() {
			return ZERO_HASH;
		}
		let mut res = None;
		for peak in self.peaks().iter().rev() {
			res = match res {
				None => Some(*peak),
				Some(rhash) => Some((*peak, rhash).hash_with_index(self.unpruned_size())),
			}
		}
		res.expect("no root, invalid tree")
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
			}).collect()
	}

	/// Total size of the tree, including intermediary nodes and ignoring any
	/// pruning.
	pub fn unpruned_size(&self) -> u64 {
		self.last_pos
	}

	/// Helper function which returns un-pruned nodes from the insertion index
	/// forward
	/// returns last insertion index returned along with data
	pub fn elements_from_insertion_index(
		&self,
		mut index: u64,
		max_count: u64,
	) -> (u64, Vec<T::E>) {
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

	/// Helper function to get the last N nodes inserted, i.e. the last
	/// n nodes along the bottom of the tree.
	/// May return less than n items if the MMR has been pruned/compacted.
	pub fn get_last_n_insertions(&self, n: u64) -> Vec<(Hash, T::E)> {
		let mut return_vec = vec![];
		let mut last_leaf = self.last_pos;
		for _ in 0..n as u64 {
			if last_leaf == 0 {
				break;
			}
			last_leaf = bintree_rightmost(last_leaf);

			if let Some(hash) = self.backend.get_hash(last_leaf) {
				if let Some(data) = self.backend.get_data(last_leaf) {
					return_vec.push((hash, data));
				}
			}
			last_leaf -= 1;
		}
		return_vec
	}
}
