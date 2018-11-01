// Copyright 2018 The Grin Developers
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

//! Implementation of the persistent Backend for the prunable MMR tree.

use std::{fs, io, marker};

use croaring::Bitmap;

use core::core::hash::{Hash, Hashed};
use core::core::pmmr::{self, family, Backend, HashOnlyBackend};
use core::core::BlockHeader;
use core::ser::{self, FixedLength, PMMRable};
use leaf_set::LeafSet;
use prune_list::PruneList;
use types::{prune_noop, AppendOnlyFile, HashFile};

const PMMR_HASH_FILE: &str = "pmmr_hash.bin";
const PMMR_DATA_FILE: &str = "pmmr_data.bin";
const PMMR_LEAF_FILE: &str = "pmmr_leaf.bin";
const PMMR_PRUN_FILE: &str = "pmmr_prun.bin";

/// The list of PMMR_Files for internal purposes
pub const PMMR_FILES: [&str; 4] = [
	PMMR_HASH_FILE,
	PMMR_DATA_FILE,
	PMMR_LEAF_FILE,
	PMMR_PRUN_FILE,
];

/// PMMR persistent backend implementation. Relies on multiple facilities to
/// handle writing, reading and pruning.
///
/// * A main storage file appends Hash instances as they come.
/// This AppendOnlyFile is also backed by a mmap for reads.
/// * An in-memory backend buffers the latest batch of writes to ensure the
/// PMMR can always read recent values even if they haven't been flushed to
/// disk yet.
/// * A leaf_set tracks unpruned (unremoved) leaf positions in the MMR..
/// * A prune_list tracks the positions of pruned (and compacted) roots in the
/// MMR.
pub struct PMMRBackend<T: PMMRable> {
	data_dir: String,
	prunable: bool,
	hash_file: AppendOnlyFile,
	data_file: AppendOnlyFile,
	leaf_set: LeafSet,
	prune_list: PruneList,
	_marker: marker::PhantomData<T>,
}

impl<T: PMMRable> Backend<T> for PMMRBackend<T> {
	/// Append the provided data and hashes to the backend storage.
	/// Add the new leaf pos to our leaf_set if this is a prunable MMR.
	#[allow(unused_variables)]
	fn append(&mut self, data: T, hashes: Vec<Hash>) -> Result<(), String> {
		if self.prunable {
			let record_len = Hash::LEN as u64;
			let shift = self.prune_list.get_total_shift();
			let position = (self.hash_file.size_unsync() / record_len) + shift + 1;
			self.leaf_set.add(position);
		}
		self.data_file.append(&mut ser::ser_vec(&data).unwrap());
		for h in &hashes {
			self.hash_file.append(&mut ser::ser_vec(h).unwrap());
		}
		Ok(())
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		if self.is_compacted(position) {
			return None;
		}

		let shift = self.prune_list.get_shift(position);

		// Read PMMR
		// The MMR starts at 1, our binary backend starts at 0
		let pos = position - 1;

		// Must be on disk, doing a read at the correct position
		let hash_record_len = Hash::LEN;
		let file_offset = ((pos - shift) as usize) * hash_record_len;
		let data = self.hash_file.read(file_offset, hash_record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(h) => Some(h),
			Err(e) => {
				error!(
					"Corrupted storage, could not read an entry from hash store: {:?}",
					e
				);
				None
			}
		}
	}

	fn get_data_from_file(&self, position: u64) -> Option<T> {
		if self.is_compacted(position) {
			return None;
		}
		let shift = self.prune_list.get_leaf_shift(position);
		let pos = pmmr::n_leaves(position) - 1;

		// Must be on disk, doing a read at the correct position
		let record_len = T::LEN;
		let file_offset = ((pos - shift) as usize) * record_len;
		let data = self.data_file.read(file_offset, record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(h) => Some(h),
			Err(e) => {
				error!(
					"Corrupted storage, could not read an entry from data store: {:?}",
					e
				);
				None
			}
		}
	}

	/// Get the hash at pos.
	/// Return None if pos is a leaf and it has been removed (or pruned or
	/// compacted).
	fn get_hash(&self, pos: u64) -> Option<(Hash)> {
		if self.prunable && pmmr::is_leaf(pos) && !self.leaf_set.includes(pos) {
			return None;
		}
		self.get_from_file(pos)
	}

	/// Get the data at pos.
	/// Return None if it has been removed or if pos is not a leaf node.
	fn get_data(&self, pos: u64) -> Option<(T)> {
		if !pmmr::is_leaf(pos) {
			return None;
		}
		if self.prunable && !self.leaf_set.includes(pos) {
			return None;
		}
		self.get_data_from_file(pos)
	}

	/// Rewind the PMMR backend to the given position.
	fn rewind(&mut self, position: u64, rewind_rm_pos: &Bitmap) -> Result<(), String> {
		// First rewind the leaf_set with the necessary added and removed positions.
		if self.prunable {
			self.leaf_set.rewind(position, rewind_rm_pos);
		}

		// Rewind the hash file accounting for pruned/compacted pos
		let shift = self.prune_list.get_shift(position);
		let record_len = Hash::LEN as u64;
		let file_pos = (position - shift) * record_len;
		self.hash_file.rewind(file_pos);

		// Rewind the data file accounting for pruned/compacted pos
		let leaf_shift = self.prune_list.get_leaf_shift(position);
		let flatfile_pos = pmmr::n_leaves(position);
		let record_len = T::LEN as u64;
		let file_pos = (flatfile_pos - leaf_shift) * record_len;
		self.data_file.rewind(file_pos);

		Ok(())
	}

	/// Remove by insertion position.
	fn remove(&mut self, pos: u64) -> Result<(), String> {
		assert!(self.prunable, "Remove on non-prunable MMR");
		self.leaf_set.remove(pos);
		Ok(())
	}

	/// Return data file path
	fn get_data_file_path(&self) -> String {
		self.data_file.path()
	}

	fn snapshot(&self, header: &BlockHeader) -> Result<(), String> {
		self.leaf_set
			.snapshot(header)
			.map_err(|_| format!("Failed to save copy of leaf_set for {}", header.hash()))?;
		Ok(())
	}

	fn dump_stats(&self) {
		debug!(
			"pmmr backend: unpruned: {}, hashes: {}, data: {}, leaf_set: {}, prune_list: {}",
			self.unpruned_size().unwrap_or(0),
			self.hash_size().unwrap_or(0),
			self.data_size().unwrap_or(0),
			self.leaf_set.len(),
			self.prune_list.len(),
		);
	}
}

impl<T: PMMRable> PMMRBackend<T> {
	/// Instantiates a new PMMR backend.
	/// Use the provided dir to store its files.
	pub fn new(
		data_dir: String,
		prunable: bool,
		header: Option<&BlockHeader>,
	) -> io::Result<PMMRBackend<T>> {
		let hash_file = AppendOnlyFile::open(&format!("{}/{}", data_dir, PMMR_HASH_FILE))?;
		let data_file = AppendOnlyFile::open(&format!("{}/{}", data_dir, PMMR_DATA_FILE))?;

		let leaf_set_path = format!("{}/{}", data_dir, PMMR_LEAF_FILE);

		// If we received a rewound "snapshot" leaf_set file move it into
		// place so we use it.
		if let Some(header) = header {
			let leaf_snapshot_path = format!("{}/{}.{}", data_dir, PMMR_LEAF_FILE, header.hash());
			LeafSet::copy_snapshot(&leaf_set_path, &leaf_snapshot_path)?;
		}

		let leaf_set = LeafSet::open(&leaf_set_path)?;
		let prune_list = PruneList::open(&format!("{}/{}", data_dir, PMMR_PRUN_FILE))?;

		Ok(PMMRBackend {
			data_dir,
			prunable,
			hash_file,
			data_file,
			leaf_set,
			prune_list,
			_marker: marker::PhantomData,
		})
	}

	fn is_pruned(&self, pos: u64) -> bool {
		self.prune_list.is_pruned(pos)
	}

	fn is_pruned_root(&self, pos: u64) -> bool {
		self.prune_list.is_pruned_root(pos)
	}

	fn is_compacted(&self, pos: u64) -> bool {
		self.is_pruned(pos) && !self.is_pruned_root(pos)
	}

	/// Number of elements in the PMMR stored by this backend. Only produces the
	/// fully sync'd size.
	pub fn unpruned_size(&self) -> io::Result<u64> {
		let total_shift = self.prune_list.get_total_shift();

		let record_len = Hash::LEN as u64;
		let sz = self.hash_file.size()?;
		Ok(sz / record_len + total_shift)
	}

	/// Number of elements in the underlying stored data. Extremely dependent on
	/// pruning and compaction.
	pub fn data_size(&self) -> io::Result<u64> {
		let record_len = T::LEN as u64;
		self.data_file.size().map(|sz| sz / record_len)
	}

	/// Size of the underlying hashed data. Extremely dependent on pruning
	/// and compaction.
	pub fn hash_size(&self) -> io::Result<u64> {
		self.hash_file.size().map(|sz| sz / Hash::LEN as u64)
	}

	/// Syncs all files to disk. A call to sync is required to ensure all the
	/// data has been successfully written to disk.
	pub fn sync(&mut self) -> io::Result<()> {
		if let Err(e) = self.hash_file.flush() {
			return Err(io::Error::new(
				io::ErrorKind::Interrupted,
				format!("Could not write to log hash storage, disk full? {:?}", e),
			));
		}
		if let Err(e) = self.data_file.flush() {
			return Err(io::Error::new(
				io::ErrorKind::Interrupted,
				format!("Could not write to log data storage, disk full? {:?}", e),
			));
		}

		// Flush the leaf_set to disk.
		self.leaf_set.flush()?;

		Ok(())
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.hash_file.discard();
		self.leaf_set.discard();
		self.data_file.discard();
	}

	/// Return the data file path
	pub fn data_file_path(&self) -> String {
		self.get_data_file_path()
	}

	/// Takes the leaf_set at a given cutoff_pos and generates an updated
	/// prune_list. Saves the updated prune_list to disk
	/// Compacts the hash and data files based on the prune_list and saves both
	/// to disk.
	///
	/// A cutoff position limits compaction on recent data.
	/// This will be the last position of a particular block
	/// to keep things aligned.
	/// The block_marker in the db/index for the particular block
	/// will have a suitable output_pos.
	/// This is used to enforce a horizon after which the local node
	/// should have all the data to allow rewinding.
	pub fn check_compact<P>(
		&mut self,
		cutoff_pos: u64,
		rewind_rm_pos: &Bitmap,
		prune_cb: P,
	) -> io::Result<bool>
	where
		P: Fn(&[u8]),
	{
		assert!(self.prunable, "Trying to compact a non-prunable PMMR");

		// Paths for tmp hash and data files.
		let tmp_prune_file_hash = format!("{}/{}.hashprune", self.data_dir, PMMR_HASH_FILE);
		let tmp_prune_file_data = format!("{}/{}.dataprune", self.data_dir, PMMR_DATA_FILE);

		// Calculate the sets of leaf positions and node positions to remove based
		// on the cutoff_pos provided.
		let (leaves_removed, pos_to_rm) = self.pos_to_rm(cutoff_pos, rewind_rm_pos);

		// 1. Save compact copy of the hash file, skipping removed data.
		{
			let record_len = Hash::LEN as u64;

			let off_to_rm = map_vec!(pos_to_rm, |pos| {
				let shift = self.prune_list.get_shift(pos.into());
				((pos as u64) - 1 - shift) * record_len
			});

			self.hash_file.save_prune(
				tmp_prune_file_hash.clone(),
				&off_to_rm,
				record_len,
				&prune_noop,
			)?;
		}

		// 2. Save compact copy of the data file, skipping removed leaves.
		{
			let record_len = T::LEN as u64;

			let leaf_pos_to_rm = pos_to_rm
				.iter()
				.filter(|&x| pmmr::is_leaf(x.into()))
				.map(|x| x as u64)
				.collect::<Vec<_>>();

			let off_to_rm = map_vec!(leaf_pos_to_rm, |&pos| {
				let flat_pos = pmmr::n_leaves(pos);
				let shift = self.prune_list.get_leaf_shift(pos);
				(flat_pos - 1 - shift) * record_len
			});

			self.data_file.save_prune(
				tmp_prune_file_data.clone(),
				&off_to_rm,
				record_len,
				prune_cb,
			)?;
		}

		// 3. Update the prune list and write to disk.
		{
			for pos in leaves_removed.iter() {
				self.prune_list.add(pos.into());
			}
			self.prune_list.flush()?;
		}

		// 4. Rename the compact copy of hash file and reopen it.
		fs::rename(
			tmp_prune_file_hash.clone(),
			format!("{}/{}", self.data_dir, PMMR_HASH_FILE),
		)?;
		self.hash_file = AppendOnlyFile::open(&format!("{}/{}", self.data_dir, PMMR_HASH_FILE))?;

		// 5. Rename the compact copy of the data file and reopen it.
		fs::rename(
			tmp_prune_file_data.clone(),
			format!("{}/{}", self.data_dir, PMMR_DATA_FILE),
		)?;
		self.data_file = AppendOnlyFile::open(&format!("{}/{}", self.data_dir, PMMR_DATA_FILE))?;

		// 6. Write the leaf_set to disk.
		// Optimize the bitmap storage in the process.
		self.leaf_set.flush()?;

		Ok(true)
	}

	fn pos_to_rm(&self, cutoff_pos: u64, rewind_rm_pos: &Bitmap) -> (Bitmap, Bitmap) {
		let mut expanded = Bitmap::create();

		let leaf_pos_to_rm =
			self.leaf_set
				.removed_pre_cutoff(cutoff_pos, rewind_rm_pos, &self.prune_list);

		for x in leaf_pos_to_rm.iter() {
			expanded.add(x);
			let mut current = x as u64;
			loop {
				let (parent, sibling) = family(current);
				let sibling_pruned = self.is_pruned_root(sibling);

				// if sibling previously pruned
				// push it back onto list of pos to remove
				// so we can remove it and traverse up to parent
				if sibling_pruned {
					expanded.add(sibling as u32);
				}

				if sibling_pruned || expanded.contains(sibling as u32) {
					expanded.add(parent as u32);
					current = parent;
				} else {
					break;
				}
			}
		}
		(leaf_pos_to_rm, removed_excl_roots(&expanded))
	}
}

/// Simple MMR Backend for hashes only (data maintained in the db).
pub struct HashOnlyMMRBackend {
	/// The hash file underlying this MMR backend.
	hash_file: HashFile,
}

impl HashOnlyBackend for HashOnlyMMRBackend {
	fn append(&mut self, hashes: Vec<Hash>) -> Result<(), String> {
		for h in &hashes {
			self.hash_file
				.append(h)
				.map_err(|e| format!("Failed to append to backend, {:?}", e))?;
		}
		Ok(())
	}

	fn rewind(&mut self, position: u64) -> Result<(), String> {
		self.hash_file
			.rewind(position)
			.map_err(|e| format!("Failed to rewind backend, {:?}", e))?;
		Ok(())
	}

	fn get_hash(&self, position: u64) -> Option<Hash> {
		self.hash_file.read(position)
	}
}

impl HashOnlyMMRBackend {
	/// Instantiates a new PMMR backend.
	/// Use the provided dir to store its files.
	pub fn new(data_dir: &str) -> io::Result<HashOnlyMMRBackend> {
		let hash_file = HashFile::open(&format!("{}/{}", data_dir, PMMR_HASH_FILE))?;
		Ok(HashOnlyMMRBackend { hash_file })
	}

	/// The unpruned size of this MMR backend.
	pub fn unpruned_size(&self) -> io::Result<u64> {
		let sz = self.hash_file.size()?;
		Ok(sz / Hash::LEN as u64)
	}

	/// Discard any pending changes to this MMR backend.
	pub fn discard(&mut self) {
		self.hash_file.discard();
	}

	/// Sync pending changes to the backend file on disk.
	pub fn sync(&mut self) -> io::Result<()> {
		if let Err(e) = self.hash_file.flush() {
			return Err(io::Error::new(
				io::ErrorKind::Interrupted,
				format!("Could not write to hash storage, disk full? {:?}", e),
			));
		}
		Ok(())
	}
}

/// Filter remove list to exclude roots.
/// We want to keep roots around so we have hashes for Merkle proofs.
fn removed_excl_roots(removed: &Bitmap) -> Bitmap {
	removed
		.iter()
		.filter(|pos| {
			let (parent_pos, _) = family(*pos as u64);
			removed.contains(parent_pos as u32)
		}).collect()
}
