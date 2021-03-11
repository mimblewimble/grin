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

use std::convert::TryFrom;
use std::{collections::HashSet, marker};

use croaring::Bitmap;

use crate::core::hash::Hash;
use crate::core::pmmr::{self, Backend};
use crate::core::BlockHeader;
use crate::ser::PMMRable;

/// Simple/minimal/naive MMR backend implementation backed by Vec<T> and Vec<Hash>.
/// Removed pos are maintained in a HashSet<u64>.
#[derive(Clone, Debug)]
pub struct VecBackend<T: PMMRable> {
	/// Vec of hashes for the PMMR (both leaves and parents).
	pub hashes: Vec<Hash>,
	/// Positions of removed elements (is this applicable if we do not store data?)
	pub removed: HashSet<u64>,
	_marker: marker::PhantomData<T>,
}

impl<T: PMMRable> Backend<T> for VecBackend<T> {
	fn append(&mut self, hashes: &[Hash]) -> Result<(), String> {
		self.hashes.extend_from_slice(hashes);
		Ok(())
	}

	fn get_hash(&self, position: u64) -> Option<Hash> {
		if self.removed.contains(&position) {
			None
		} else {
			self.get_from_file(position)
		}
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		let idx = usize::try_from(position.saturating_sub(1)).expect("usize from u64");
		self.hashes.get(idx).cloned()
	}

	fn get_peak_from_file(&self, position: u64) -> Option<Hash> {
		self.get_from_file(position)
	}

	/// Number of leaves in the MMR
	fn n_unpruned_leaves(&self) -> u64 {
		unimplemented!()
	}

	fn leaf_pos_iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
		Box::new(
			self.hashes
				.iter()
				.enumerate()
				.map(|(x, _)| (x + 1) as u64)
				.filter(move |x| pmmr::is_leaf(*x) && !self.removed.contains(x)),
		)
	}

	fn leaf_idx_iter(&self, from_idx: u64) -> Box<dyn Iterator<Item = u64> + '_> {
		let from_pos = pmmr::insertion_to_pmmr_index(from_idx + 1);
		Box::new(
			self.leaf_pos_iter()
				.skip_while(move |x| *x < from_pos)
				.map(|x| pmmr::n_leaves(x).saturating_sub(1)),
		)
	}

	fn is_leaf(&self, pos: u64) -> bool {
		pmmr::is_leaf(pos) && !self.removed.contains(&pos)
	}

	fn remove(&mut self, position: u64) -> Result<(), String> {
		self.removed.insert(position);
		Ok(())
	}

	fn rewind(&mut self, position: u64, _rewind_rm_pos: &Bitmap) -> Result<(), String> {
		self.hashes
			.truncate(usize::try_from(position).expect("usize from u64"));
		Ok(())
	}

	fn snapshot(&self, _header: &BlockHeader) -> Result<(), String> {
		Ok(())
	}

	fn release_files(&mut self) {}

	fn dump_stats(&self) {}
}

impl<T: PMMRable> VecBackend<T> {
	/// Instantiates a new empty vec backend.
	pub fn new() -> VecBackend<T> {
		VecBackend {
			hashes: vec![],
			removed: HashSet::new(),
			_marker: marker::PhantomData,
		}
	}

	/// Size of this vec backend in hashes.
	pub fn size(&self) -> u64 {
		self.hashes.len() as u64
	}
}
