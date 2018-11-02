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
use core::pmmr::{is_leaf, peaks, Backend};
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

	/// Is the MMR empty?
	pub fn is_empty(&self) -> bool {
		self.last_pos == 0
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

	/// Total size of the tree, including intermediary nodes and ignoring any
	/// pruning.
	pub fn unpruned_size(&self) -> u64 {
		self.last_pos
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

	/// Get the data element at provided position in the MMR.
	pub fn get_data(&self, pos: u64) -> Option<T> {
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
}
