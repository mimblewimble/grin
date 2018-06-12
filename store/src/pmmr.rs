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

use croaring::Bitmap;

use core::core::hash::Hash;
use core::core::pmmr::{self, family, Backend};
use core::ser;
use core::ser::PMMRable;
use types::*;
use util::LOGGER;
use utxo_set::UtxoSet;

const PMMR_HASH_FILE: &'static str = "pmmr_hash.bin";
const PMMR_DATA_FILE: &'static str = "pmmr_data.bin";
const PMMR_UTXO_FILE: &'static str = "pmmr_utxo.bin";
const PMMR_PRUNED_FILE: &'static str = "pmmr_pruned.bin";

/// Maximum number of nodes in the remove log before it gets flushed
pub const RM_LOG_MAX_NODES: usize = 10_000;

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
	pub utxo_set: UtxoSet,
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

				// Add the new position to our UTXO set.
				self.utxo_set.add(position);
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
	/// Return None if pos is a leaf and it has been spent.
	fn get_hash(&self, pos: u64) -> Option<(Hash)> {
		if pmmr::is_leaf(pos) && !self.utxo_set.includes(pos) {
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
		if !self.utxo_set.includes(pos) {
			return None;
		}
		self.get_data_from_file(pos)
	}

	/// Rewind the PMMR backend to the given position.
	fn rewind(
		&mut self,
		position: u64,
		rewind_output_pos: &Bitmap,
		rewind_spent_pos: &Bitmap,
	) -> Result<(), String> {
		// First rewind the UTXO set with the pos of outputs and spent outputs (inputs).
		self.utxo_set.rewind(rewind_output_pos, rewind_spent_pos);

		// Rewind the hash file accounting for pruned/compacted pos
		let shift = self.pruned_nodes.get_shift(position).unwrap_or(0);
		let record_len = 32 as u64;
		let file_pos = (position - shift) * record_len;
		self.hash_file.rewind(file_pos);

		// Rewind the data file accounting for pruned/compacted pos
		let leaf_shift = self.pruned_nodes.get_leaf_shift(position).unwrap_or(0);
		let flatfile_pos = pmmr::n_leaves(position);
		let record_len = T::len() as u64;
		let file_pos = (flatfile_pos - leaf_shift) * record_len;
		self.data_file.rewind(file_pos);

		Ok(())
	}

	/// Remove by insertion position.
	fn remove(&mut self, pos: u64) -> Result<(), String> {
		self.utxo_set.remove(pos);
		Ok(())
	}

	/// Return data file path
	fn get_data_file_path(&self) -> String {
		self.data_file.path()
	}

	fn dump_stats(&self) {
		debug!(
			LOGGER,
			"pmmr backend: unpruned: {}, hashes: {}, data: {}, utxo_set: {}, prune_list: {}",
			self.unpruned_size().unwrap_or(0),
			self.hash_size().unwrap_or(0),
			self.data_size().unwrap_or(0),
			self.utxo_set.len(),
			self.pruned_nodes.pruned_nodes.len(),
		);
	}
}

impl<T> PMMRBackend<T>
where
	T: PMMRable + ::std::fmt::Debug,
{
	/// Instantiates a new PMMR backend.
	/// Use the provided dir to store its files.
	pub fn new(data_dir: String) -> io::Result<PMMRBackend<T>> {
		let prune_list = read_ordered_vec(format!("{}/{}", data_dir, PMMR_PRUNED_FILE), 8)?;
		let pruned_nodes = pmmr::PruneList {
			pruned_nodes: prune_list,
		};
		let utxo_set = UtxoSet::open(format!("{}/{}", data_dir, PMMR_UTXO_FILE))?;
		let hash_file = AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_HASH_FILE))?;
		let data_file = AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_DATA_FILE))?;

		Ok(PMMRBackend {
			data_dir,
			hash_file,
			data_file,
			utxo_set,
			pruned_nodes,
			_marker: marker::PhantomData,
		})
	}

	fn is_pruned(&self, pos: u64) -> bool {
		let path = pmmr::path(pos, self.unpruned_size().unwrap_or(0));
		path.iter()
			.any(|x| self.pruned_nodes.pruned_nodes.contains(x))
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
		self.utxo_set.flush()?;

		Ok(())
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.hash_file.discard();
		self.utxo_set.discard();
		self.data_file.discard();
	}

	/// Return the data file path
	pub fn data_file_path(&self) -> String {
		self.get_data_file_path()
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
		cutoff_pos: u64,
		prune_cb: P,
	) -> io::Result<bool>
	where
		P: Fn(&[u8]),
	{
		// TODO - what do we check here for deciding to compact or not?
		// if !(max_len > 0 && self.rm_log.len() >= max_len
		// 	|| max_len == 0 && self.rm_log.len() > RM_LOG_MAX_NODES)
		// {
		// 	return Ok(false);
		// }

		// Paths for tmp hash and data files.
		let tmp_prune_file_hash = format!("{}/{}.hashprune", self.data_dir, PMMR_HASH_FILE);
		let tmp_prune_file_data = format!("{}/{}.dataprune", self.data_dir, PMMR_DATA_FILE);

		let (leaves_removed, pos_to_rm) = self.pos_to_rm(cutoff_pos);

		// 1. Save compact copy of the hash file, skipping removed data.
		{
			let record_len = 32;

			let off_to_rm = map_vec!(pos_to_rm, |pos| {
				let shift = self.pruned_nodes.get_shift(pos.into()).unwrap();
				((pos as u64) - 1 - shift) * record_len
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

			let leaf_pos_to_rm = pos_to_rm
				.iter()
				.filter(|&x| pmmr::is_leaf(x.into()))
				.map(|x| x as u64)
				.collect::<Vec<_>>();

			let off_to_rm = map_vec!(leaf_pos_to_rm, |&pos| {
				let flat_pos = pmmr::n_leaves(pos);
				let shift = self.pruned_nodes.get_leaf_shift(pos).unwrap();
				(flat_pos - 1 - shift) * record_len
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
			for pos in leaves_removed.iter() {
				self.pruned_nodes.add(pos.into());
			}
			// TODO - we can get rid of leaves in the prunelist here (and things still work)
			// self.pruned_nodes.pruned_nodes.retain(|&x| !pmmr::is_leaf(x));

			// Prunelist contains *only* non-leaf roots.
			// Contrast this with the UTXO set that contains *only* leaves.
			self.pruned_nodes
				.pruned_nodes
				.retain(|&x| !pmmr::is_leaf(x));

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
		self.hash_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_HASH_FILE))?;

		// 5. Rename the compact copy of the data file and reopen it.
		fs::rename(
			tmp_prune_file_data.clone(),
			format!("{}/{}", self.data_dir, PMMR_DATA_FILE),
		)?;
		self.data_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_DATA_FILE))?;

		// 6. Write the UTXO set to disk.
		// (optimizing the roaring bitmap storage in the process).
		self.utxo_set.flush()?;

		Ok(true)
	}

	fn pos_to_rm(&self, cutoff_pos: u64) -> (Bitmap, Bitmap) {
		let mut expanded = Bitmap::create();

		let leaf_pos_to_rm = self.utxo_set.spent_lte_pos(cutoff_pos, &self.pruned_nodes);

		for x in leaf_pos_to_rm.iter() {
			expanded.add(x);
			let mut current = x as u64;
			loop {
				let (parent, sibling) = family(current);
				let sibling_pruned = self.is_pruned(sibling);

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
		(leaf_pos_to_rm, removed_excl_roots(expanded))
	}
}

/// Filter remove list to exclude roots.
/// We want to keep roots around so we have hashes for Merkle proofs.
fn removed_excl_roots(removed: Bitmap) -> Bitmap {
	removed
		.iter()
		.filter(|pos| {
			let (parent_pos, _) = family(*pos as u64);
			removed.contains(parent_pos as u32)
		})
		.collect()
}
