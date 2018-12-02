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

use croaring::Bitmap;

use core::hash::Hash;
use core::BlockHeader;
use ser::PMMRable;

/// Storage backend for the MMR, just needs to be indexed by order of insertion.
/// The PMMR itself does not need the Backend to be accurate on the existence
/// of an element (i.e. remove could be a no-op) but layers above can
/// depend on an accurate Backend to check existence.
pub trait Backend<T: PMMRable> {
	/// Append the provided Hashes to the backend storage, and optionally an
	/// associated data element to flatfile storage (for leaf nodes only). The
	/// position of the first element of the Vec in the MMR is provided to
	/// help the implementation.
	fn append(&mut self, data: &T, hashes: Vec<Hash>) -> Result<(), String>;

	/// Rewind the backend state to a previous position, as if all append
	/// operations after that had been canceled. Expects a position in the PMMR
	/// to rewind to as well as bitmaps representing the positions added and
	/// removed since the rewind position. These are what we will "undo"
	/// during the rewind.
	fn rewind(&mut self, position: u64, rewind_rm_pos: &Bitmap) -> Result<(), String>;

	/// Get a Hash by insertion position.
	fn get_hash(&self, position: u64) -> Option<Hash>;

	/// Get underlying data by insertion position.
	fn get_data(&self, position: u64) -> Option<T::E>;

	/// Get a Hash  by original insertion position
	/// (ignoring the remove log).
	fn get_from_file(&self, position: u64) -> Option<Hash>;

	/// Get a Data Element by original insertion position
	/// (ignoring the remove log).
	fn get_data_from_file(&self, position: u64) -> Option<T::E>;

	/// Remove Hash by insertion position. An index is also provided so the
	/// underlying backend can implement some rollback of positions up to a
	/// given index (practically the index is the height of a block that
	/// triggered removal).
	fn remove(&mut self, position: u64) -> Result<(), String>;

	/// Returns the data file path.. this is a bit of a hack now that doesn't
	/// sit well with the design, but TxKernels have to be summed and the
	/// fastest way to to be able to allow direct access to the file
	fn get_data_file_path(&self) -> String;

	/// Also a bit of a hack...
	/// Saves a snapshot of the rewound utxo file with the block hash as
	/// filename suffix. We need this when sending a txhashset zip file to a
	/// node for fast sync.
	fn snapshot(&self, header: &BlockHeader) -> Result<(), String>;

	/// For debugging purposes so we can see how compaction is doing.
	fn dump_stats(&self);
}
