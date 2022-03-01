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

use std::collections::HashSet;
use std::convert::TryFrom;

use croaring::Bitmap;

use crate::core::hash::Hash;
use crate::core::pmmr::{self, Backend};
use crate::core::BlockHeader;
use crate::ser::PMMRable;

/// Simple/minimal/naive MMR backend implementation backed by Vec<T> and Vec<Hash>.
/// Removed pos are maintained in a HashSet<u64>.
#[derive(Clone, Debug)]
pub struct VecBackend<T: PMMRable> {
	/// Backend elements (optional, possible to just store hashes).
	pub data: Option<Vec<T>>,
	/// Vec of hashes for the PMMR (both leaves and parents).
	pub hashes: Vec<Hash>,
	/// Positions of removed elements (is this applicable if we do not store data?)
	pub removed: HashSet<u64>,
}

impl<T: PMMRable> Backend<T> for VecBackend<T> {
	fn append(&mut self, elmt: &T, hashes: &[Hash]) -> Result<(), String> {
		if let Some(data) = &mut self.data {
			data.push(elmt.clone());
		}
		self.hashes.extend_from_slice(hashes);
		Ok(())
	}

	fn append_pruned_subtree(&mut self, _hash: Hash, _pos0: u64) -> Result<(), String> {
		unimplemented!()
	}

	fn append_hash(&mut self, _hash: Hash) -> Result<(), String> {
		unimplemented!()
	}

	fn get_hash(&self, pos0: u64) -> Option<Hash> {
		if self.removed.contains(&pos0) {
			None
		} else {
			self.get_from_file(pos0)
		}
	}

	fn get_data(&self, pos0: u64) -> Option<T::E> {
		if self.removed.contains(&pos0) {
			None
		} else {
			self.get_data_from_file(pos0)
		}
	}

	fn get_from_file(&self, pos0: u64) -> Option<Hash> {
		let idx = usize::try_from(pos0).expect("usize from u64");
		self.hashes.get(idx).cloned()
	}

	fn get_peak_from_file(&self, pos0: u64) -> Option<Hash> {
		self.get_from_file(pos0)
	}

	fn get_data_from_file(&self, pos0: u64) -> Option<T::E> {
		if let Some(data) = &self.data {
			let idx = usize::try_from(pmmr::n_leaves(1 + pos0) - 1).expect("usize from u64");
			data.get(idx).map(|x| x.as_elmt())
		} else {
			None
		}
	}

	/// Number of leaves in the MMR
	fn n_unpruned_leaves(&self) -> u64 {
		unimplemented!()
	}

	fn n_unpruned_leaves_to_index(&self, _to_index: u64) -> u64 {
		unimplemented!()
	}

	fn leaf_pos_iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
		Box::new(
			self.hashes
				.iter()
				.enumerate()
				.map(|(x, _)| x as u64)
				.filter(move |x| pmmr::is_leaf(*x) && !self.removed.contains(x)),
		)
	}

	/// NOTE this function is needlessly inefficient with repeated calls to n_leaves()
	fn leaf_idx_iter(&self, from_idx: u64) -> Box<dyn Iterator<Item = u64> + '_> {
		let from_pos = pmmr::insertion_to_pmmr_index(from_idx);
		Box::new(
			self.leaf_pos_iter()
				.skip_while(move |x| *x < from_pos)
				.map(|x| pmmr::n_leaves(x + 1) - 1),
		)
	}

	fn remove(&mut self, pos0: u64) -> Result<(), String> {
		self.removed.insert(pos0);
		Ok(())
	}

	fn remove_from_leaf_set(&mut self, _pos0: u64) {
		unimplemented!()
	}

	fn reset_prune_list(&mut self) {
		unimplemented!()
	}

	fn rewind(&mut self, position: u64, _rewind_rm_pos: &Bitmap) -> Result<(), String> {
		if let Some(data) = &mut self.data {
			let idx = pmmr::n_leaves(position);
			data.truncate(usize::try_from(idx).expect("usize from u64"));
		}
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
			data: Some(vec![]),
			hashes: vec![],
			removed: HashSet::new(),
		}
	}

	/// Instantiate a new empty "hash only" vec backend.
	pub fn new_hash_only() -> VecBackend<T> {
		VecBackend {
			data: None,
			hashes: vec![],
			removed: HashSet::new(),
		}
	}

	/// Size of this vec backend in hashes.
	pub fn size(&self) -> u64 {
		self.hashes.len() as u64
	}
}
