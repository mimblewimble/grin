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

use std::fs;
use std::io;
use std::marker;

use core::core::pmmr::{self, family, Backend};
use core::ser::{self, PMMRable, Readable, Reader, Writeable, Writer};
use core::core::hash::Hash;
use util::LOGGER;
use types::*;

const PMMR_HASH_FILE: &'static str = "pmmr_hash.bin";
const PMMR_DATA_FILE: &'static str = "pmmr_data.bin";
const PMMR_RM_LOG_FILE: &'static str = "pmmr_rm_log.bin";
const PMMR_PRUNED_FILE: &'static str = "pmmr_pruned.bin";

/// Maximum number of nodes in the remove log before it gets flushed
pub const RM_LOG_MAX_NODES: usize = 10_000;

/// Metadata for the PMMR backend's AppendOnlyFile, which can be serialized and
/// stored
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PMMRFileMetadata {
	/// The block height represented by these indices in the file
	pub block_height: u64,
	/// last written index of the hash file
	pub last_hash_file_pos: u64,
	/// last written index of the data file
	pub last_data_file_pos: u64,
}

impl Writeable for PMMRFileMetadata {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.block_height)?;
		writer.write_u64(self.last_hash_file_pos)?;
		writer.write_u64(self.last_data_file_pos)?;
		Ok(())
	}
}

impl Readable for PMMRFileMetadata {
	fn read(reader: &mut Reader) -> Result<PMMRFileMetadata, ser::Error> {
		Ok(PMMRFileMetadata {
			block_height: reader.read_u64()?,
			last_hash_file_pos: reader.read_u64()?,
			last_data_file_pos: reader.read_u64()?,
		})
	}
}

impl PMMRFileMetadata {
	/// Return fields with all positions = 0
	pub fn empty() -> PMMRFileMetadata {
		PMMRFileMetadata {
			block_height: 0,
			last_hash_file_pos: 0,
			last_data_file_pos: 0,
		}
	}
}

/// PMMR persistent backend implementation. Relies on multiple facilities to
/// handle writing, reading and pruning.
///
/// * A main storage file appends Hash instances as they come. This
/// AppendOnlyFile is also backed by a mmap for reads.
/// * An in-memory backend buffers the latest batch of writes to ensure the
/// PMMR can always read recent values even if they haven't been flushed to
/// disk yet.
/// * A remove log tracks the positions that need to be pruned from the
/// main storage file.
pub struct PMMRBackend<T>
where
	T: PMMRable,
{
	data_dir: String,
	hash_file: AppendOnlyFile,
	data_file: AppendOnlyFile,
	rm_log: RemoveLog,
	pruned_nodes: pmmr::PruneList,
	_marker: marker::PhantomData<T>,
}

impl<T> Backend<T> for PMMRBackend<T>
where
	T: PMMRable + ::std::fmt::Debug,
{
	/// Append the provided Hashes to the backend storage.
	#[allow(unused_variables)]
	fn append(&mut self, position: u64, data: Vec<(Hash, Option<T>)>) -> Result<(), String> {
		for d in data {
			self.hash_file.append(&mut ser::ser_vec(&d.0).unwrap());
			if let Some(elem) = d.1 {
				self.data_file.append(&mut ser::ser_vec(&elem).unwrap());
			}
		}
		Ok(())
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		let shift = self.pruned_nodes.get_shift(position);
		if let None = shift {
			return None;
		}

		// Read PMMR
		// The MMR starts at 1, our binary backend starts at 0
		let pos = position - 1;

		// Must be on disk, doing a read at the correct position
		let hash_record_len = 32;
		let file_offset = ((pos - shift.unwrap()) as usize) * hash_record_len;
		let data = self.hash_file.read(file_offset, hash_record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(h) => Some(h),
			Err(e) => {
				error!(
					LOGGER,
					"Corrupted storage, could not read an entry from hash store: {:?}", e
				);
				return None;
			}
		}
	}

	fn get_data_from_file(&self, position: u64) -> Option<T> {
		let shift = self.pruned_nodes.get_leaf_shift(position);
		if let None = shift {
			return None;
		}

		let pos = pmmr::n_leaves(position) - 1;

		// Must be on disk, doing a read at the correct position
		let record_len = T::len();
		let file_offset = ((pos - shift.unwrap()) as usize) * record_len;
		let data = self.data_file.read(file_offset, record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(h) => Some(h),
			Err(e) => {
				error!(
					LOGGER,
					"Corrupted storage, could not read an entry from data store: {:?}", e
				);
				return None;
			}
		}
	}

	/// Get the hash at pos.
	/// Return None if it has been removed.
	fn get_hash(&self, pos: u64) -> Option<(Hash)> {
		// Check if this position has been pruned in the remove log...
		if self.rm_log.includes(pos) {
			None
		} else {
			self.get_from_file(pos)
		}
	}

	/// Get the data at pos.
	/// Return None if it has been removed or if pos is not a leaf node.
	fn get_data(&self, pos: u64) -> Option<(T)> {
		if self.rm_log.includes(pos) {
			None
		} else if !pmmr::is_leaf(pos) {
			None
		} else {
			self.get_data_from_file(pos)
		}
	}

	fn rewind(&mut self, position: u64, index: u32) -> Result<(), String> {
		self.rm_log
			.rewind(index)
			.map_err(|e| format!("Could not truncate remove log: {}", e))?;

		// Hash file
		let shift = self.pruned_nodes.get_shift(position).unwrap_or(0);
		let record_len = 32;
		let file_pos = (position - shift) * (record_len as u64);
		self.hash_file.rewind(file_pos);

		// Data file
		let leaf_shift = self.pruned_nodes.get_leaf_shift(position).unwrap_or(0);
		let flatfile_pos = pmmr::n_leaves(position);
		let file_pos = (flatfile_pos - leaf_shift) * T::len() as u64;
		self.data_file.rewind(file_pos as u64);
		Ok(())
	}

	/// Remove Hashes by insertion position
	fn remove(&mut self, positions: Vec<u64>, index: u32) -> Result<(), String> {
		self.rm_log
			.append(positions, index)
			.map_err(|e| format!("Could not write to log storage, disk full? {:?}", e))
	}

	/// Return data file path
	fn get_data_file_path(&self) -> String {
		self.data_file.path()
	}

	fn dump_stats(&self) {
		debug!(
			LOGGER,
			"pmmr backend: unpruned - {}, hashes - {}, data - {}, rm_log - {:?}",
			self.unpruned_size().unwrap_or(0),
			self.hash_size().unwrap_or(0),
			self.data_size().unwrap_or(0),
			self.rm_log.removed
		);
	}
}

impl<T> PMMRBackend<T>
where
	T: PMMRable + ::std::fmt::Debug,
{
	/// Instantiates a new PMMR backend that will use the provided directly to
	/// store its files.
	pub fn new(data_dir: String, file_md: Option<PMMRFileMetadata>) -> io::Result<PMMRBackend<T>> {
		let (height, hash_to_pos, data_to_pos) = match file_md {
			Some(m) => (
				m.block_height as u32,
				m.last_hash_file_pos,
				m.last_data_file_pos,
			),
			None => (0, 0, 0),
		};
		let hash_file =
			AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_HASH_FILE), hash_to_pos)?;
		let rm_log = RemoveLog::open(format!("{}/{}", data_dir, PMMR_RM_LOG_FILE), height)?;
		let prune_list = read_ordered_vec(format!("{}/{}", data_dir, PMMR_PRUNED_FILE), 8)?;
		let data_file =
			AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_DATA_FILE), data_to_pos)?;

		Ok(PMMRBackend {
			data_dir: data_dir,
			hash_file: hash_file,
			data_file: data_file,
			rm_log: rm_log,
			pruned_nodes: pmmr::PruneList {
				pruned_nodes: prune_list,
			},
			_marker: marker::PhantomData,
		})
	}

	/// Number of elements in the PMMR stored by this backend. Only produces the
	/// fully sync'd size.
	pub fn unpruned_size(&self) -> io::Result<u64> {
		let total_shift = self.pruned_nodes.get_shift(::std::u64::MAX).unwrap();

		let record_len = 32;
		let sz = self.hash_file.size()?;
		Ok(sz / record_len + total_shift)
	}

	/// Number of elements in the underlying stored data. Extremely dependent on
	/// pruning and compaction.
	pub fn data_size(&self) -> io::Result<u64> {
		let record_len = T::len() as u64;
		self.data_file.size().map(|sz| sz / record_len)
	}

	/// Size of the underlying hashed data. Extremely dependent on pruning
	/// and compaction.
	pub fn hash_size(&self) -> io::Result<u64> {
		self.hash_file.size().map(|sz| sz / 32)
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
		self.rm_log.flush()?;

		Ok(())
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.hash_file.discard();
		self.rm_log.discard();
		self.data_file.discard();
	}

	/// Return the data file path
	pub fn data_file_path(&self) -> String {
		self.get_data_file_path()
	}

	/// Return last written buffer positions for the hash file and the data file
	pub fn last_file_positions(&self) -> PMMRFileMetadata {
		PMMRFileMetadata {
			block_height: 0,
			last_hash_file_pos: self.hash_file.last_buffer_pos() as u64,
			last_data_file_pos: self.data_file.last_buffer_pos() as u64,
		}
	}

	/// Checks the length of the remove log to see if it should get compacted.
	/// If so, the remove log is flushed into the pruned list, which itself gets
	/// saved, and the hash and data files are rewritten, cutting the removed
	/// data.
	///
	/// If a max_len strictly greater than 0 is provided, the value will be used
	/// to decide whether the remove log has reached its maximum length,
	/// otherwise the RM_LOG_MAX_NODES default value is used.
	///
	/// A cutoff limits compaction on recent data. Provided as an indexed value
	/// on pruned data (practically a block height), it forces compaction to
	/// ignore any prunable data beyond the cutoff. This is used to enforce
	/// a horizon after which the local node should have all the data to allow
	/// rewinding.
	pub fn check_compact<P>(
		&mut self,
		max_len: usize,
		cutoff_index: u32,
		prune_cb: P,
	) -> io::Result<bool>
	where
		P: Fn(&[u8]),
	{
		if !(max_len > 0 && self.rm_log.len() >= max_len
			|| max_len == 0 && self.rm_log.len() > RM_LOG_MAX_NODES)
		{
			return Ok(false);
		}

		// Paths for tmp hash and data files.
		let tmp_prune_file_hash = format!("{}/{}.hashprune", self.data_dir, PMMR_HASH_FILE);
		let tmp_prune_file_data = format!("{}/{}.dataprune", self.data_dir, PMMR_DATA_FILE);

		// Pos we want to get rid of.
		// Filtered by cutoff index.
		let rm_pre_cutoff = self.rm_log.removed_pre_cutoff(cutoff_index);
		// Filtered to exclude the subtree "roots".
		let pos_to_rm = removed_excl_roots(rm_pre_cutoff.clone());
		// Filtered for leaves only.
		let leaf_pos_to_rm = removed_leaves(pos_to_rm.clone());

		// 1. Save compact copy of the hash file, skipping removed data.
		{
			let record_len = 32;

			let off_to_rm = map_vec!(pos_to_rm, |&pos| {
				let shift = self.pruned_nodes.get_shift(pos).unwrap();
				(pos - 1 - shift) * record_len
			});

			self.hash_file.save_prune(
				tmp_prune_file_hash.clone(),
				off_to_rm,
				record_len,
				&prune_noop,
			)?;
		}

		// 2. Save compact copy of the data file, skipping removed leaves.
		{
			let record_len = T::len() as u64;

			let off_to_rm = map_vec!(leaf_pos_to_rm, |pos| {
				let shift = self.pruned_nodes.get_leaf_shift(*pos);
				(pmmr::n_leaves(pos - shift.unwrap()) - 1) * record_len
			});

			self.data_file.save_prune(
				tmp_prune_file_data.clone(),
				off_to_rm,
				record_len,
				prune_cb,
			)?;
		}

		// 3. Update the prune list and save it in place.
		{
			for &pos in &rm_pre_cutoff {
				self.pruned_nodes.add(pos);
			}

			write_vec(
				format!("{}/{}", self.data_dir, PMMR_PRUNED_FILE),
				&self.pruned_nodes.pruned_nodes,
			)?;
		}

		// 4. Rename the compact copy of hash file and reopen it.
		fs::rename(
			tmp_prune_file_hash.clone(),
			format!("{}/{}", self.data_dir, PMMR_HASH_FILE),
		)?;
		self.hash_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_HASH_FILE), 0)?;

		// 5. Rename the compact copy of the data file and reopen it.
		fs::rename(
			tmp_prune_file_data.clone(),
			format!("{}/{}", self.data_dir, PMMR_DATA_FILE),
		)?;
		self.data_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_DATA_FILE), 0)?;

		// 6. Truncate the rm log based on pos removed.
		// Excluding roots which remain in rm log.
		self.rm_log
			.removed
			.retain(|&(pos, _)| !pos_to_rm.binary_search(&&pos).is_ok());
		self.rm_log.flush()?;

		Ok(true)
	}
}

/// Filter remove list to exclude roots.
/// We want to keep roots around so we have hashes for Merkle proofs.
fn removed_excl_roots(removed: Vec<u64>) -> Vec<u64> {
	removed
		.iter()
		.filter(|&pos| {
			let (parent_pos, _) = family(*pos);
			removed.binary_search(&parent_pos).is_ok()
		})
		.cloned()
		.collect()
}

/// Filter remove list to only include leaf positions.
fn removed_leaves(removed: Vec<u64>) -> Vec<u64> {
	removed
		.iter()
		.filter(|&pos| pmmr::is_leaf(*pos))
		.cloned()
		.collect()
}
