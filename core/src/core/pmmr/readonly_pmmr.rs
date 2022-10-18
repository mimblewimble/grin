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

//! Readonly view of a PMMR.

use std::marker;

use crate::core::hash::Hash;
use crate::core::pmmr::pmmr::{bintree_rightmost, ReadablePMMR};
use crate::core::pmmr::{is_leaf, Backend};
use crate::ser::PMMRable;

/// Readonly view of a PMMR.
pub struct ReadonlyPMMR<'a, T, B>
where
	T: PMMRable,
	B: Backend<T>,
{
	/// The last position in the PMMR
	size: u64,
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
	pub fn new(backend: &'a B) -> ReadonlyPMMR<'_, T, B> {
		ReadonlyPMMR {
			backend,
			size: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new readonly PMMR pre-initialized to
	/// size with the provided backend.
	pub fn at(backend: &'a B, size: u64) -> ReadonlyPMMR<'_, T, B> {
		ReadonlyPMMR {
			backend,
			size,
			_marker: marker::PhantomData,
		}
	}

	/// Helper function which returns un-pruned nodes from the insertion index
	/// forward
	/// returns last pmmr index returned along with data
	pub fn elements_from_pmmr_index(
		&self,
		pmmr_index1: u64,
		max_count: u64,
		max_pmmr_pos1: Option<u64>,
	) -> (u64, Vec<T::E>) {
		let mut return_vec = vec![];
		let size = match max_pmmr_pos1 {
			Some(p) => p,
			None => self.size,
		};
		let mut pmmr_index = pmmr_index1.saturating_sub(1);

		while return_vec.len() < max_count as usize && pmmr_index < size {
			if let Some(t) = self.get_data(pmmr_index) {
				return_vec.push(t);
			}
			pmmr_index += 1;
		}
		(pmmr_index, return_vec)
	}

	/// Helper function to get the last N nodes inserted, i.e. the last
	/// n nodes along the bottom of the tree.
	/// May return less than n items if the MMR has been pruned/compacted.
	/// NOTE This should just iterate over insertion indices
	/// to avoid the repeated calls to bintree_rightmost!
	pub fn get_last_n_insertions(&self, n: u64) -> Vec<(Hash, T::E)> {
		let mut return_vec = vec![];
		let mut last_leaf = self.size;
		while return_vec.len() < n as usize && last_leaf > 0 {
			last_leaf = bintree_rightmost(last_leaf - 1);

			if let Some(hash) = self.backend.get_hash(last_leaf) {
				if let Some(data) = self.backend.get_data(last_leaf) {
					return_vec.push((hash, data));
				}
			}
		}
		return_vec
	}
}

impl<'a, T, B> ReadablePMMR for ReadonlyPMMR<'a, T, B>
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
