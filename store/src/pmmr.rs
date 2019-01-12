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

use std::{fs, io, time};

use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::pmmr::{self, family, Backend};
use crate::core::core::BlockHeader;
use crate::core::ser::PMMRable;
use crate::leaf_set::LeafSet;
use crate::prune_list::PruneList;
use crate::types::{prune_noop, DataFile};
use croaring::Bitmap;
use std::path::{Path, PathBuf};

const PMMR_HASH_FILE: &str = "pmmr_hash.bin";
const PMMR_DATA_FILE: &str = "pmmr_data.bin";
const PMMR_LEAF_FILE: &str = "pmmr_leaf.bin";
const PMMR_PRUN_FILE: &str = "pmmr_prun.bin";
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
	#[allow(unused_variables)]
	fn append(&mut self, data: &T, hashes: Vec<Hash>) -> Result<(), String> {
		if self.prunable {
			let shift = self.prune_list.get_total_shift();
			let position = self.hash_file.size_unsync() + shift + 1;
			self.leaf_set.add(position);
		}

		self.data_file
			.append(&data.as_elmt())
			.map_err(|e| format!("Failed to append data to file. {}", e))?;

		for h in &hashes {
			self.hash_file
				.append(h)
				.map_err(|e| format!("Failed to append hash to file. {}", e))?;
		}
		Ok(())
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		if self.is_compacted(position) {
			return None;
		}
		let shift = self.prune_list.get_shift(position);
		self.hash_file.read(position - shift)
	}

	fn get_data_from_file(&self, position: u64) -> Option<T::E> {
		if self.is_compacted(position) {
			return None;
		}
		let flatfile_pos = pmmr::n_leaves(position);
		let shift = self.prune_list.get_leaf_shift(position);
		self.data_file.read(flatfile_pos - shift)
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
	fn get_data(&self, pos: u64) -> Option<(T::E)> {
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
		self.hash_file.rewind(position - shift);

		// Rewind the data file accounting for pruned/compacted pos
		let flatfile_pos = pmmr::n_leaves(position);
		let leaf_shift = self.prune_list.get_leaf_shift(position);
		self.data_file.rewind(flatfile_pos - leaf_shift);

		Ok(())
	}

	/// Remove by insertion position.
	fn remove(&mut self, pos: u64) -> Result<(), String> {
		assert!(self.prunable, "Remove on non-prunable MMR");
		self.leaf_set.remove(pos);
		Ok(())
	}

	/// Return data file path
	fn get_data_file_path(&self) -> &Path {
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
	/// Use the provided dir to store its files.
	pub fn new<P: AsRef<Path>>(
		data_dir: P,
		prunable: bool,
		header: Option<&BlockHeader>,
	) -> io::Result<PMMRBackend<T>> {
		let data_dir = data_dir.as_ref();
		let hash_file = DataFile::open(&data_dir.join(PMMR_HASH_FILE))?;
		let data_file = DataFile::open(&data_dir.join(PMMR_DATA_FILE))?;

		let leaf_set_path = data_dir.join(PMMR_LEAF_FILE);

		// If we received a rewound "snapshot" leaf_set file move it into
		// place so we use it.
		if let Some(header) = header {
			let leaf_snapshot_path = format!(
				"{}.{}",
				data_dir.join(PMMR_LEAF_FILE).to_str().unwrap(),
				header.hash()
			);
			// Check for a ... (3 dot) ending version of the file - could probably be removed after mainnet
			let compatible_snapshot_path = PathBuf::from(leaf_snapshot_path.clone() + "...");
			if compatible_snapshot_path.exists() {
				LeafSet::copy_snapshot(&leaf_set_path, &compatible_snapshot_path)?;
			} else {
				LeafSet::copy_snapshot(&leaf_set_path, &PathBuf::from(leaf_snapshot_path))?;
			}
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
	pub fn unpruned_size(&self) -> u64 {
		let total_shift = self.prune_list.get_total_shift();
		let sz = self.hash_file.size();
		sz + total_shift
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
		self.hash_file
			.flush()
			.and(self.data_file.flush())
			.and(self.leaf_set.flush())
			.map_err(|e| {
				io::Error::new(
					io::ErrorKind::Interrupted,
					format!("Could not write to state storage, disk full? {:?}", e),
				)
			})
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.hash_file.discard();
		self.leaf_set.discard();
		self.data_file.discard();
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
		let tmp_prune_file_hash =
			format!("{}.hashprune", self.data_dir.join(PMMR_HASH_FILE).display());
		let tmp_prune_file_data =
			format!("{}.dataprune", self.data_dir.join(PMMR_DATA_FILE).display());
		// Calculate the sets of leaf positions and node positions to remove based
		// on the cutoff_pos provided.
		let (leaves_removed, pos_to_rm) = self.pos_to_rm(cutoff_pos, rewind_rm_pos);

		// 1. Save compact copy of the hash file, skipping removed data.
		{
			let off_to_rm = map_vec!(pos_to_rm, |pos| {
				let shift = self.prune_list.get_shift(pos.into());
				pos as u64 - 1 - shift
			});

			self.hash_file
				.save_prune(&tmp_prune_file_hash, &off_to_rm, &prune_noop)?;
		}

		// 2. Save compact copy of the data file, skipping removed leaves.
		{
			let leaf_pos_to_rm = pos_to_rm
				.iter()
				.filter(|&x| pmmr::is_leaf(x.into()))
				.map(|x| x as u64)
				.collect::<Vec<_>>();

			let off_to_rm = map_vec!(leaf_pos_to_rm, |&pos| {
				let flat_pos = pmmr::n_leaves(pos);
				let shift = self.prune_list.get_leaf_shift(pos);
				(flat_pos - 1 - shift)
			});

			self.data_file
				.save_prune(&tmp_prune_file_data, &off_to_rm, prune_cb)?;
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
			self.data_dir.join(PMMR_HASH_FILE),
		)?;
		self.hash_file = DataFile::open(self.data_dir.join(PMMR_HASH_FILE))?;

		// 5. Rename the compact copy of the data file and reopen it.
		fs::rename(
			tmp_prune_file_data.clone(),
			self.data_dir.join(PMMR_DATA_FILE),
		)?;
		self.data_file = DataFile::open(self.data_dir.join(PMMR_DATA_FILE))?;

		// 6. Write the leaf_set to disk.
		// Optimize the bitmap storage in the process.
		self.leaf_set.flush()?;

		// 7. cleanup rewind files
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

/// Filter remove list to exclude roots.
/// We want to keep roots around so we have hashes for Merkle proofs.
fn removed_excl_roots(removed: &Bitmap) -> Bitmap {
	removed
		.iter()
		.filter(|pos| {
			let (parent_pos, _) = family(*pos as u64);
			removed.contains(parent_pos as u32)
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
			|possible_dir_entry| -> Result<u32, Box<std::error::Error>> {
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
