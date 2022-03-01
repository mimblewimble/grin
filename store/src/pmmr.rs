// Copyright 2021 The Grin Developers
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

use std::fs;
use std::{io, time};

use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::pmmr::{self, family, Backend};
use crate::core::core::BlockHeader;
use crate::core::ser::{PMMRable, ProtocolVersion};
use crate::leaf_set::LeafSet;
use crate::prune_list::PruneList;
use crate::types::{AppendOnlyFile, DataFile, SizeEntry, SizeInfo};
use croaring::Bitmap;
use std::convert::TryInto;
use std::path::{Path, PathBuf};

const PMMR_HASH_FILE: &str = "pmmr_hash.bin";
const PMMR_DATA_FILE: &str = "pmmr_data.bin";
const PMMR_LEAF_FILE: &str = "pmmr_leaf.bin";
const PMMR_PRUN_FILE: &str = "pmmr_prun.bin";
const PMMR_SIZE_FILE: &str = "pmmr_size.bin";
const REWIND_FILE_CLEANUP_DURATION_SECONDS: u64 = 60 * 60 * 24; // 24 hours as seconds

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
	data_dir: PathBuf,
	prunable: bool,
	hash_file: DataFile<Hash>,
	data_file: DataFile<T::E>,
	leaf_set: LeafSet,
	prune_list: PruneList,
}

impl<T: PMMRable> Backend<T> for PMMRBackend<T> {
	/// Append the provided data and hashes to the backend storage.
	/// Add the new leaf pos to our leaf_set if this is a prunable MMR.
	fn append(&mut self, data: &T, hashes: &[Hash]) -> Result<(), String> {
		let size = self
			.data_file
			.append(&data.as_elmt())
			.map_err(|e| format!("Failed to append data to file. {}", e))?;

		self.hash_file
			.extend_from_slice(hashes)
			.map_err(|e| format!("Failed to append hash to file. {}", e))?;

		if self.prunable {
			// (Re)calculate the latest pos given updated size of data file
			// and the total leaf_shift, and add to our leaf_set.
			let pos =
				pmmr::insertion_to_pmmr_index(size + self.prune_list.get_total_leaf_shift() - 1);
			self.leaf_set.add(pos);
		}

		Ok(())
	}

	// Supports appending a pruned subtree (single root hash) to an existing hash file.
	// Update the prune_list "shift cache" to reflect the new pruned leaf pos in the subtree.
	fn append_pruned_subtree(&mut self, hash: Hash, pos0: u64) -> Result<(), String> {
		if !self.prunable {
			return Err("Not prunable, cannot append pruned subtree.".into());
		}

		self.hash_file
			.append(&hash)
			.map_err(|e| format!("Failed to append subtree hash to file. {}", e))?;

		self.prune_list.append(pos0);

		Ok(())
	}

	fn append_hash(&mut self, hash: Hash) -> Result<(), String> {
		self.hash_file
			.append(&hash)
			.map_err(|e| format!("Failed to append hash to file. {}", e))?;
		Ok(())
	}

	fn get_from_file(&self, pos0: u64) -> Option<Hash> {
		if self.is_compacted(pos0) {
			return None;
		}
		let shift = self.prune_list.get_shift(pos0);
		self.hash_file.read(1 + pos0 - shift)
	}

	fn get_peak_from_file(&self, pos0: u64) -> Option<Hash> {
		let shift = self.prune_list.get_shift(pos0);
		self.hash_file.read(1 + pos0 - shift)
	}

	fn get_data_from_file(&self, pos0: u64) -> Option<T::E> {
		if !pmmr::is_leaf(pos0) {
			return None;
		}
		if self.is_compacted(pos0) {
			return None;
		}
		let flatfile_pos = pmmr::n_leaves(pos0 + 1);
		let shift = self.prune_list.get_leaf_shift(1 + pos0);
		self.data_file.read(flatfile_pos - shift)
	}

	/// Get the hash at pos.
	/// Return None if pos is a leaf and it has been removed (or pruned or
	/// compacted).
	fn get_hash(&self, pos0: u64) -> Option<Hash> {
		if self.prunable && pmmr::is_leaf(pos0) && !self.leaf_set.includes(pos0) {
			return None;
		}
		self.get_from_file(pos0)
	}

	/// Get the data at pos.
	/// Return None if it has been removed or if pos is not a leaf node.
	fn get_data(&self, pos0: u64) -> Option<T::E> {
		if !pmmr::is_leaf(pos0) {
			return None;
		}
		if self.prunable && !self.leaf_set.includes(pos0) {
			return None;
		}
		self.get_data_from_file(pos0)
	}

	/// Remove leaf from leaf set
	fn remove_from_leaf_set(&mut self, pos0: u64) {
		self.leaf_set.remove(pos0);
	}

	/// Returns an iterator over all the leaf positions.
	/// for a prunable PMMR this is an iterator over the leaf_set bitmap.
	/// For a non-prunable PMMR this is *all* leaves (this is not yet implemented).
	fn leaf_pos_iter(&self) -> Box<dyn Iterator<Item = u64> + '_> {
		if self.prunable {
			Box::new(self.leaf_set.iter().map(|x| x - 1))
		} else {
			panic!("leaf_pos_iter not implemented for non-prunable PMMR")
		}
	}

	fn n_unpruned_leaves(&self) -> u64 {
		if self.prunable {
			self.leaf_set.len() as u64
		} else {
			pmmr::n_leaves(self.unpruned_size())
		}
	}

	fn n_unpruned_leaves_to_index(&self, to_index: u64) -> u64 {
		if self.prunable {
			self.leaf_set.n_unpruned_leaves_to_index(to_index)
		} else {
			pmmr::n_leaves(pmmr::insertion_to_pmmr_index(to_index))
		}
	}

	/// Returns an iterator over all the leaf insertion indices (0-indexed).
	/// If our pos are [1,2,4,5,8] (first 5 leaf pos) then our insertion indices are [0,1,2,3,4]
	fn leaf_idx_iter(&self, from_idx: u64) -> Box<dyn Iterator<Item = u64> + '_> {
		// pass from_idx in as param
		// convert this to pos
		// iterate, skipping everything prior to this
		// pass in from_idx=0 then we want to convert to pos=1

		let from_pos = 1 + pmmr::insertion_to_pmmr_index(from_idx);

		if self.prunable {
			Box::new(
				self.leaf_set
					.iter()
					.skip_while(move |x| *x < from_pos)
					.map(|x| pmmr::n_leaves(x).saturating_sub(1)),
			)
		} else {
			panic!("leaf_idx_iter not implemented for non-prunable PMMR")
		}
	}

	/// Rewind the PMMR backend to the given position.
	fn rewind(&mut self, position: u64, rewind_rm_pos: &Bitmap) -> Result<(), String> {
		// First rewind the leaf_set with the necessary added and removed positions.
		if self.prunable {
			self.leaf_set.rewind(position, rewind_rm_pos);
		}

		// Rewind the hash file accounting for pruned/compacted pos
		let shift = if position == 0 {
			0
		} else {
			self.prune_list.get_shift(position - 1)
		};
		self.hash_file.rewind(position - shift);

		// Rewind the data file accounting for pruned/compacted pos
		let flatfile_pos = pmmr::n_leaves(position);
		let leaf_shift = if position == 0 {
			0
		} else {
			self.prune_list.get_leaf_shift(position)
		};
		self.data_file.rewind(flatfile_pos - leaf_shift);

		Ok(())
	}

	fn reset_prune_list(&mut self) {
		let bitmap = Bitmap::create();
		self.prune_list = PruneList::new(Some(self.data_dir.join(PMMR_PRUN_FILE)), bitmap);
		if let Err(e) = self.prune_list.flush() {
			error!("Flushing reset prune list: {}", e);
		}
	}

	/// Remove by insertion position.
	fn remove(&mut self, pos0: u64) -> Result<(), String> {
		assert!(self.prunable, "Remove on non-prunable MMR");
		self.leaf_set.remove(pos0);
		Ok(())
	}

	/// Release underlying data files
	fn release_files(&mut self) {
		self.data_file.release();
		self.hash_file.release();
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
			self.unpruned_size(),
			self.hash_size(),
			self.data_size(),
			self.leaf_set.len(),
			self.prune_list.len(),
		);
	}
}

impl<T: PMMRable> PMMRBackend<T> {
	/// Instantiates a new PMMR backend.
	/// If optional size is provided then treat as "fixed" size otherwise "variable" size backend.
	/// Use the provided dir to store its files.
	pub fn new<P: AsRef<Path>>(
		data_dir: P,
		prunable: bool,
		version: ProtocolVersion,
		header: Option<&BlockHeader>,
	) -> io::Result<PMMRBackend<T>> {
		let data_dir = data_dir.as_ref();

		// Are we dealing with "fixed size" data elements or "variable size" data elements
		// maintained in an associated size file?
		let size_info = if let Some(fixed_size) = T::elmt_size() {
			SizeInfo::FixedSize(fixed_size)
		} else {
			SizeInfo::VariableSize(Box::new(AppendOnlyFile::open(
				data_dir.join(PMMR_SIZE_FILE),
				SizeInfo::FixedSize(SizeEntry::LEN as u16),
				version,
			)?))
		};

		// Hash file is always "fixed size" and we use 32 bytes per hash.
		let hash_size_info = SizeInfo::FixedSize(Hash::LEN.try_into().unwrap());

		let hash_file = DataFile::open(&data_dir.join(PMMR_HASH_FILE), hash_size_info, version)?;
		let data_file = DataFile::open(&data_dir.join(PMMR_DATA_FILE), size_info, version)?;

		let leaf_set_path = data_dir.join(PMMR_LEAF_FILE);

		// If we received a rewound "snapshot" leaf_set file move it into
		// place so we use it.
		if let Some(header) = header {
			let leaf_snapshot_path = format!(
				"{}.{}",
				data_dir.join(PMMR_LEAF_FILE).to_str().unwrap(),
				header.hash()
			);
			LeafSet::copy_snapshot(&leaf_set_path, &PathBuf::from(leaf_snapshot_path))?;
		}

		let leaf_set = LeafSet::open(&leaf_set_path)?;
		let prune_list = PruneList::open(&data_dir.join(PMMR_PRUN_FILE))?;

		Ok(PMMRBackend {
			data_dir: data_dir.to_path_buf(),
			prunable,
			hash_file,
			data_file,
			leaf_set,
			prune_list,
		})
	}

	fn is_pruned(&self, pos0: u64) -> bool {
		self.prune_list.is_pruned(pos0)
	}

	fn is_pruned_root(&self, pos0: u64) -> bool {
		self.prune_list.is_pruned_root(pos0)
	}

	// Check if pos is pruned but not a pruned root itself.
	// Checking for pruned root is faster so we do this check first.
	// We can do a fast initial check as well -
	// if its in the current leaf_set then we know it is not compacted.
	fn is_compacted(&self, pos0: u64) -> bool {
		if self.leaf_set.includes(pos0) {
			return false;
		}
		!self.is_pruned_root(pos0) && self.is_pruned(pos0)
	}

	/// Number of hashes in the PMMR stored by this backend. Only produces the
	/// fully sync'd size.
	pub fn unpruned_size(&self) -> u64 {
		self.hash_size() + self.prune_list.get_total_shift()
	}

	/// Number of elements in the underlying stored data. Extremely dependent on
	/// pruning and compaction.
	pub fn data_size(&self) -> u64 {
		self.data_file.size()
	}

	/// Size of the underlying hashed data. Extremely dependent on pruning
	/// and compaction.
	pub fn hash_size(&self) -> u64 {
		self.hash_file.size()
	}

	/// Syncs all files to disk. A call to sync is required to ensure all the
	/// data has been successfully written to disk.
	pub fn sync(&mut self) -> io::Result<()> {
		Ok(())
			.and(self.hash_file.flush())
			.and(self.data_file.flush())
			.and(self.sync_leaf_set())
			.and(self.prune_list.flush())
			.map_err(|e| {
				io::Error::new(
					io::ErrorKind::Interrupted,
					format!("Could not sync pmmr to disk: {:?}", e),
				)
			})
	}

	// Sync the leaf_set if this is a prunable backend.
	fn sync_leaf_set(&mut self) -> io::Result<()> {
		if !self.prunable {
			return Ok(());
		}
		self.leaf_set.flush()
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.hash_file.discard();
		self.data_file.discard();
		self.leaf_set.discard();
	}

	/// Takes the leaf_set at a given cutoff_pos and generates an updated
	/// prune_list. Saves the updated prune_list to disk, compacts the hash
	/// and data files based on the prune_list and saves both to disk.
	///
	/// A cutoff position limits compaction on recent data.
	/// This will be the last position of a particular block to keep things
	/// aligned. The block_marker in the db/index for the particular block
	/// will have a suitable output_pos. This is used to enforce a horizon
	/// after which the local node should have all the data to allow rewinding.
	pub fn check_compact(&mut self, cutoff_pos: u64, rewind_rm_pos: &Bitmap) -> io::Result<bool> {
		assert!(self.prunable, "Trying to compact a non-prunable PMMR");

		// Calculate the sets of leaf positions and node positions to remove based
		// on the cutoff_pos provided.
		let (leaves_removed, pos_to_rm) = self.pos_to_rm(cutoff_pos, rewind_rm_pos);

		// Save compact copy of the hash file, skipping removed data.
		{
			let pos_to_rm = map_vec!(pos_to_rm, |pos1| {
				let shift = self.prune_list.get_shift(pos1 as u64 - 1);
				pos1 as u64 - shift
			});

			self.hash_file.write_tmp_pruned(&pos_to_rm)?;
		}

		// Save compact copy of the data file, skipping removed leaves.
		{
			let leaf_pos_to_rm = pos_to_rm
				.iter()
				.map(|x| x as u64)
				.filter(|x| pmmr::is_leaf(x - 1))
				.collect::<Vec<_>>();

			let pos_to_rm = map_vec!(leaf_pos_to_rm, |&pos| {
				let flat_pos = pmmr::n_leaves(pos);
				let shift = self.prune_list.get_leaf_shift(pos);
				flat_pos - shift
			});

			self.data_file.write_tmp_pruned(&pos_to_rm)?;
		}

		// Replace hash and data files with compact copies.
		// Rebuild and intialize from the new files.
		{
			debug!("compact: about to replace hash and data files and rebuild...");
			self.hash_file.replace_with_tmp()?;
			self.data_file.replace_with_tmp()?;
			debug!("compact: ...finished replacing and rebuilding");
		}

		// Update the prune list and write to disk.
		{
			let mut bitmap = self.prune_list.bitmap();
			bitmap.or_inplace(&leaves_removed);
			self.prune_list = PruneList::new(Some(self.data_dir.join(PMMR_PRUN_FILE)), bitmap);
			self.prune_list.flush()?;
		}

		// Write the leaf_set to disk.
		// Optimize the bitmap storage in the process.
		self.leaf_set.flush()?;

		self.clean_rewind_files()?;

		Ok(true)
	}

	fn clean_rewind_files(&self) -> io::Result<u32> {
		let data_dir = self.data_dir.clone();
		let pattern = format!("{}.", PMMR_LEAF_FILE);
		clean_files_by_prefix(data_dir, &pattern, REWIND_FILE_CLEANUP_DURATION_SECONDS)
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
				let (parent0, sibling0) = family(current - 1);
				let sibling_pruned = self.is_pruned_root(sibling0);

				// if sibling previously pruned
				// push it back onto list of pos to remove
				// so we can remove it and traverse up to parent
				if sibling_pruned {
					expanded.add(1 + sibling0 as u32);
				}

				if sibling_pruned || expanded.contains(1 + sibling0 as u32) {
					expanded.add(1 + parent0 as u32);
					current = 1 + parent0;
				} else {
					break;
				}
			}
		}
		(leaf_pos_to_rm, removed_excl_roots(&expanded))
	}
}

/// Filter remove list to exclude roots.
/// We want to keep roots around so we have hashes for Merkle proofs.
fn removed_excl_roots(removed: &Bitmap) -> Bitmap {
	removed
		.iter()
		.filter(|pos| {
			let (parent_pos0, _) = family(*pos as u64 - 1);
			removed.contains(1 + parent_pos0 as u32)
		})
		.collect()
}

/// Quietly clean a directory up based on a given prefix.
/// If the file was accessed within cleanup_duration_seconds from the beginning of
/// the function call, it will not be deleted. To delete all files, set cleanup_duration_seconds
/// to zero.
///
/// Precondition is that path points to a directory.
///
/// If you have files such as
/// ```text
/// foo
/// foo.1
/// foo.2
/// .
/// .
/// .
/// .
/// .
/// ```
///
/// call this function and you will get
///
/// ```text
/// foo
/// ```
///
/// in the directory
///
/// The return value will be the number of files that were deleted.
///
/// This function will return an error whenever the call to `std;:fs::read_dir`
/// fails on the given path for any reason.
///

pub fn clean_files_by_prefix<P: AsRef<std::path::Path>>(
	path: P,
	prefix_to_delete: &str,
	cleanup_duration_seconds: u64,
) -> io::Result<u32> {
	let now = time::SystemTime::now();
	let cleanup_duration = time::Duration::from_secs(cleanup_duration_seconds);

	let number_of_files_deleted: u32 = fs::read_dir(&path)?
		.flat_map(
			|possible_dir_entry| -> Result<u32, Box<dyn std::error::Error>> {
				// result implements iterator and so if we were to use map here
				// we would have a list of Result<u32, Box<std::error::Error>>
				// but because we use flat_map, the errors get "discarded" and we are
				// left with a clean iterator over u32s

				// the error cases that come out of this code are numerous and
				// we don't really mind throwing them away because the main point
				// here is to clean up some files, if it doesn't work out it's not
				// the end of the world

				let dir_entry: std::fs::DirEntry = possible_dir_entry?;
				let metadata = dir_entry.metadata()?;
				if metadata.is_dir() {
					return Ok(0); // skip directories unconditionally
				}
				let accessed = metadata.accessed()?;
				let duration_since_accessed = now.duration_since(accessed)?;
				if duration_since_accessed <= cleanup_duration {
					return Ok(0); // these files are still too new
				}
				let file_name = dir_entry
					.file_name()
					.into_string()
					.ok()
					.ok_or("could not convert filename into utf-8")?;

				// check to see if we want to delete this file?
				if file_name.starts_with(prefix_to_delete)
					&& file_name.len() > prefix_to_delete.len()
				{
					// we want to delete it, try to do so
					if fs::remove_file(dir_entry.path()).is_ok() {
						// we successfully deleted a file
						return Ok(1);
					}
				}

				// we either did not want to delete this file or could
				// not for whatever reason. 0 files deleted.
				Ok(0)
			},
		)
		.sum();

	Ok(number_of_files_deleted)
}
