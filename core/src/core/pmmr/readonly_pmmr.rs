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

//! Readonly view of a PMMR.

use std::marker;

use crate::core::hash::Hash;
use crate::core::pmmr::pmmr::ReadablePMMR;
use crate::core::pmmr::{is_leaf, Backend};
use crate::ser::PMMRable;

/// Readonly view of a PMMR.
pub struct ReadonlyPMMR<'a, T, B>
where
	T: PMMRable,
	B: Backend<T>,
{
	/// The last position in the PMMR
	pub last_pos: u64,
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
			last_pos: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new readonly PMMR pre-initialized to
	/// last_pos with the provided backend.
	pub fn at(backend: &'a B, last_pos: u64) -> ReadonlyPMMR<'_, T, B> {
		ReadonlyPMMR {
			backend,
			last_pos,
			_marker: marker::PhantomData,
		}
	}

	/// Helper function which returns un-pruned nodes from the insertion index
	/// forward
	/// returns last pmmr index returned along with data
	pub fn elements_from_pmmr_index(
		&self,
		pmmr_index: u64,
		max_count: u64,
		max_pmmr_pos: Option<u64>,
	) -> (u64, Vec<T::E>) {
		panic!("implement me");
	}
	// 	let mut return_vec = vec![];
	// 	let last_pos = match max_pmmr_pos {
	// 		Some(p) => p,
	// 		None => self.last_pos,
	// 	};
	// 	if pmmr_index == 0 {
	// 		pmmr_index = 1;
	// 	}
	// 	while return_vec.len() < max_count as usize && pmmr_index <= last_pos {
	// 		if let Some(t) = self.get_data(pmmr_index) {
	// 			return_vec.push(t);
	// 		}
	// 		pmmr_index += 1;
	// 	}
	// 	(pmmr_index.saturating_sub(1), return_vec)
	// }

	/// Helper function to get the last N nodes inserted, i.e. the last
	/// n nodes along the bottom of the tree.
	/// May return less than n items if the MMR has been pruned/compacted.
	pub fn get_last_n_insertions(&self, n: u64) -> Vec<(Hash, T::E)> {
		panic!("implement me");
	}
	// 	let mut return_vec = vec![];
	// 	let mut last_leaf = self.last_pos;
	// 	for _ in 0..n as u64 {
	// 		if last_leaf == 0 {
	// 			break;
	// 		}
	// 		last_leaf = bintree_rightmost(last_leaf);

	// 		if let Some(hash) = self.backend.get_hash(last_leaf) {
	// 			if let Some(data) = self.backend.get_data(last_leaf) {
	// 				return_vec.push((hash, data));
	// 			}
	// 		}
	// 		last_leaf -= 1;
	// 	}
	// 	return_vec
	// }
}

impl<'a, T, B> ReadablePMMR for ReadonlyPMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	type Item = T::E;

	fn get_hash(&self, pos: u64) -> Option<Hash> {
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

	fn get_from_file(&self, pos: u64) -> Option<Hash> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_from_file(pos)
		}
	}

	fn get_peak_from_file(&self, pos: u64) -> Option<Hash> {
		if pos > self.last_pos {
			None
		} else {
			self.backend.get_peak_from_file(pos)
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

	fn is_leaf(&self, pos: u64) -> bool {
		pos <= self.last_pos && is_leaf(pos) && self.backend.is_leaf(pos)
	}

	fn n_unpruned_leaves(&self) -> u64 {
		self.backend.n_unpruned_leaves()
	}
}
