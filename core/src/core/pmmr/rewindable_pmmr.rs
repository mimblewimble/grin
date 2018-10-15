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

//! Rewindable (but still readonly) view of a PMMR.
//! Only supports non-pruneable backends (i.e. kernel MMR backend).

use std::marker;

use core::hash::Hash;
use core::pmmr::{bintree_postorder_height, is_leaf, peaks, Backend};
use ser::{PMMRIndexHashable, PMMRable};

/// Rewindable (but still readonly) view of a PMMR.
pub struct RewindablePMMR<'a, T, B>
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

impl<'a, T, B> RewindablePMMR<'a, T, B>
where
	T: PMMRable + ::std::fmt::Debug,
	B: 'a + Backend<T>,
{
	/// Build a new readonly PMMR.
	pub fn new(backend: &'a B) -> RewindablePMMR<T, B> {
		RewindablePMMR {
			last_pos: 0,
			backend: backend,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new readonly PMMR pre-initialized to
	/// last_pos with the provided backend.
	pub fn at(backend: &'a B, last_pos: u64) -> RewindablePMMR<T, B> {
		RewindablePMMR {
			last_pos: last_pos,
			backend: backend,
			_marker: marker::PhantomData,
		}
	}

	/// Note: We only rewind the last_pos, we do not rewind the (readonly) backend.
	/// Prunable backends are not supported here.
	pub fn rewind(&mut self, position: u64) -> Result<(), String> {
		// Identify which actual position we should rewind to as the provided
		// position is a leaf. We traverse the MMR to include any parent(s) that
		// need to be included for the MMR to be valid.
		let mut pos = position;
		while bintree_postorder_height(pos + 1) > 0 {
			pos += 1;
		}

		self.last_pos = pos;
		Ok(())
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
}
