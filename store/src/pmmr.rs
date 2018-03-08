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
use std::marker::PhantomData;

use core::core::pmmr::{self, Backend};
use core::ser::{self, PMMRable, Readable, Reader, Writeable, Writer};
use core::core::hash::Hash;
use util::LOGGER;
use types::*;

const PMMR_HASH_FILE: &'static str = "pmmr_hash.bin";
const PMMR_DATA_FILE: &'static str = "pmmr_data.bin";
const PMMR_RM_LOG_FILE: &'static str = "pmmr_rm_log.bin";
const PMMR_PRUNED_FILE: &'static str = "pmmr_pruned.bin";

/// Maximum number of nodes in the remove log before it gets flushed
pub const RM_LOG_MAX_NODES: usize = 10000;

/// Metadata for the PMMR backend's AppendOnlyFile, which can be serialized and
/// stored
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PMMRFileMetadata {
	/// last written index of the hash file
	pub last_hash_file_pos: u64,
	/// last written index of the data file
	pub last_data_file_pos: u64,
}

impl Writeable for PMMRFileMetadata {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.last_hash_file_pos)?;
		writer.write_u64(self.last_data_file_pos)?;
		Ok(())
	}
}

impl Readable for PMMRFileMetadata {
	fn read(reader: &mut Reader) -> Result<PMMRFileMetadata, ser::Error> {
		Ok(PMMRFileMetadata {
			last_hash_file_pos: reader.read_u64()?,
			last_data_file_pos: reader.read_u64()?,
		})
	}
}

impl PMMRFileMetadata {
	/// Return fields with all positions = 0
	pub fn empty() -> PMMRFileMetadata {
		PMMRFileMetadata {
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
	phantom: PhantomData<T>,
}

impl<T> Backend<T> for PMMRBackend<T>
where
	T: PMMRable,
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

	/// Get a Hash by insertion position
	fn get(&self, position: u64, include_data: bool) -> Option<(Hash, Option<T>)> {
		// Check if this position has been pruned in the remove log...
		if self.rm_log.includes(position) {
			return None;
		}
		// ... or in the prune list
		let prune_shift = match self.pruned_nodes.get_leaf_shift(position) {
			Some(shift) => shift,
			None => return None,
		};

		let hash_val = self.get_from_file(position);
		if !include_data {
			return hash_val.map(|hash| (hash, None));
		}

		// Optionally read flatfile storage to get data element
		let flatfile_pos = pmmr::n_leaves(position) - 1 - prune_shift;
		let record_len = T::len();
		let file_offset = flatfile_pos as usize * T::len();
		let data = self.data_file.read(file_offset, record_len);
		let data = match ser::deserialize(&mut &data[..]) {
			Ok(elem) => Some(elem),
			Err(e) => {
				error!(
					LOGGER,
					"Corrupted storage, could not read an entry from backend flatfile store: {:?}",
					e
				);
				None
			}
		};

		// TODO - clean this up
		if let Some(hash) = hash_val {
			return Some((hash, data));
		} else {
			return None;
		}
	}

	fn rewind(&mut self, position: u64, index: u32) -> Result<(), String> {
		self.rm_log
			.rewind(index)
			.map_err(|e| format!("Could not truncate remove log: {}", e))?;

		let shift = self.pruned_nodes.get_shift(position).unwrap_or(0);
		let record_len = 32;
		let file_pos = (position - shift) * (record_len as u64);
		self.hash_file.rewind(file_pos);

		//Data file
		let flatfile_pos = pmmr::n_leaves(position) - 1;
		let file_pos = (flatfile_pos as usize + 1) * T::len();
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
}

impl<T> PMMRBackend<T>
where
	T: PMMRable,
{
	/// Instantiates a new PMMR backend that will use the provided directly to
	/// store its files.
	pub fn new(data_dir: String, file_md: Option<PMMRFileMetadata>) -> io::Result<PMMRBackend<T>> {
		let (hash_to_pos, data_to_pos) = match file_md {
			Some(m) => (m.last_hash_file_pos, m.last_data_file_pos),
			None => (0, 0),
		};
		let hash_file =
			AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_HASH_FILE), hash_to_pos)?;
		let rm_log = RemoveLog::open(format!("{}/{}", data_dir, PMMR_RM_LOG_FILE))?;
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
			phantom: PhantomData,
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

		// 0. validate none of the nodes in the rm log are in the prune list (to
		// avoid accidental double compaction)
		for pos in &self.rm_log.removed[..] {
			if let None = self.pruned_nodes.pruned_pos(pos.0) {
				// TODO we likely can recover from this by directly jumping to 3
				error!(
					LOGGER,
					"The remove log contains nodes that are already in the pruned \
					 list, a previous compaction likely failed."
				);
				return Ok(false);
			}
		}

		// 1. save hash file to a compact copy, skipping data that's in the
		// remove list
		let tmp_prune_file_hash = format!("{}/{}.hashprune", self.data_dir, PMMR_HASH_FILE);
		let record_len = 32;
		let to_rm = filter_map_vec!(self.rm_log.removed, |&(pos, idx)| if idx < cutoff_index {
			let shift = self.pruned_nodes.get_shift(pos);
			Some((pos - 1 - shift.unwrap()) * record_len)
		} else {
			None
		});
		self.hash_file
			.save_prune(tmp_prune_file_hash.clone(), to_rm, record_len, &prune_noop)?;

		// 2. And the same with the data file
		let tmp_prune_file_data = format!("{}/{}.dataprune", self.data_dir, PMMR_DATA_FILE);
		let record_len = T::len() as u64;
		let to_rm = filter_map_vec!(self.rm_log.removed, |&(pos, idx)| {
			if pmmr::bintree_postorder_height(pos) == 0 && idx < cutoff_index {
				let shift = self.pruned_nodes.get_leaf_shift(pos).unwrap();
				let pos = pmmr::n_leaves(pos as u64);
				Some((pos - 1 - shift) * record_len)
			} else {
				None
			}
		});
		self.data_file
			.save_prune(tmp_prune_file_data.clone(), to_rm, record_len, prune_cb)?;

		// 3. update the prune list and save it in place
		for &(rm_pos, idx) in &self.rm_log.removed[..] {
			if idx < cutoff_index {
				self.pruned_nodes.add(rm_pos);
			}
		}
		write_vec(
			format!("{}/{}", self.data_dir, PMMR_PRUNED_FILE),
			&self.pruned_nodes.pruned_nodes,
		)?;

		// 4. move the compact copy of hashes to the hash file and re-open it
		fs::rename(
			tmp_prune_file_hash.clone(),
			format!("{}/{}", self.data_dir, PMMR_HASH_FILE),
		)?;
		self.hash_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_HASH_FILE), 0)?;

		// 5. and the same with the data file
		fs::rename(
			tmp_prune_file_data.clone(),
			format!("{}/{}", self.data_dir, PMMR_DATA_FILE),
		)?;
		self.data_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_DATA_FILE), 0)?;

		// 6. truncate the rm log
		self.rm_log.removed = self.rm_log
			.removed
			.iter()
			.filter(|&&(_, idx)| idx >= cutoff_index)
			.map(|x| *x)
			.collect();
		self.rm_log.flush()?;

		Ok(true)
	}
}
