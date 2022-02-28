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

use croaring::Bitmap;

use crate::core::hash::Hash;
use crate::core::BlockHeader;
use crate::ser::PMMRable;

/// Storage backend for the MMR, just needs to be indexed by order of insertion.
/// The PMMR itself does not need the Backend to be accurate on the existence
/// of an element (i.e. remove could be a no-op) but layers above can
/// depend on an accurate Backend to check existence.
pub trait Backend<T: PMMRable> {
	/// Append the provided Hashes to the backend storage, and optionally an
	/// associated data element to flatfile storage (for leaf nodes only). The
	/// position of the first element of the Vec in the MMR is provided to
	/// help the implementation.
	fn append(&mut self, data: &T, hashes: &[Hash]) -> Result<(), String>;

	/// Rebuilding a PMMR locally from PIBD segments requires pruned subtree support.
	/// This allows us to append an existing pruned subtree directly without the underlying leaf nodes.
	fn append_pruned_subtree(&mut self, hash: Hash, pos0: u64) -> Result<(), String>;

	/// Append a single hash to the pmmr
	fn append_hash(&mut self, hash: Hash) -> Result<(), String>;

	/// Rewind the backend state to a previous position, as if all append
	/// operations after that had been canceled. Expects a position in the PMMR
	/// to rewind to as well as bitmaps representing the positions added and
	/// removed since the rewind position. These are what we will "undo"
	/// during the rewind.
	fn rewind(&mut self, pos1: u64, rewind_rm_pos: &Bitmap) -> Result<(), String>;

	/// Get a Hash by insertion position.
	fn get_hash(&self, pos0: u64) -> Option<Hash>;

	/// Get underlying data by insertion position.
	fn get_data(&self, pos0: u64) -> Option<T::E>;

	/// Get a Hash  by original insertion position
	/// (ignoring the remove log).
	fn get_from_file(&self, pos0: u64) -> Option<Hash>;

	/// Get hash for peak pos.
	/// Optimized for reading peak hashes rather than arbitrary pos hashes.
	/// Peaks can be assumed to not be compacted.
	fn get_peak_from_file(&self, pos0: u64) -> Option<Hash>;

	/// Get a Data Element by original insertion position
	/// (ignoring the remove log).
	fn get_data_from_file(&self, pos0: u64) -> Option<T::E>;

	/// Iterator over current (unpruned, unremoved) leaf positions.
	fn leaf_pos_iter(&self) -> Box<dyn Iterator<Item = u64> + '_>;

	/// Number of leaves
	fn n_unpruned_leaves(&self) -> u64;

	/// Number of leaves up to the given leaf index
	fn n_unpruned_leaves_to_index(&self, to_index: u64) -> u64;

	/// Iterator over current (unpruned, unremoved) leaf insertion index.
	/// Note: This differs from underlying MMR pos - [0, 1, 2, 3, 4] vs. [1, 2, 4, 5, 8].
	fn leaf_idx_iter(&self, from_idx: u64) -> Box<dyn Iterator<Item = u64> + '_>;

	/// Remove Hash by insertion position. An index is also provided so the
	/// underlying backend can implement some rollback of positions up to a
	/// given index (practically the index is the height of a block that
	/// triggered removal).
	fn remove(&mut self, position: u64) -> Result<(), String>;

	/// Remove a leaf from the leaf set
	fn remove_from_leaf_set(&mut self, pos0: u64);

	/// Release underlying datafiles and locks
	fn release_files(&mut self);

	/// Reset prune list, used when PIBD is reset
	fn reset_prune_list(&mut self);

	/// Saves a snapshot of the rewound utxo file with the block hash as
	/// filename suffix. We need this when sending a txhashset zip file to a
	/// node for fast sync.
	fn snapshot(&self, header: &BlockHeader) -> Result<(), String>;

	/// For debugging purposes so we can see how compaction is doing.
	fn dump_stats(&self);
}
