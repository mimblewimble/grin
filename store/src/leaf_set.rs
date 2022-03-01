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

//! The Grin leaf_set implementation.
//! Compact (roaring) bitmap representing the set of leaf positions
//! that exist and are not currently pruned in the MMR.

use std::path::{Path, PathBuf};

use croaring::Bitmap;

use crate::core::core::hash::Hashed;
use crate::core::core::pmmr;
use crate::core::core::BlockHeader;
use crate::prune_list::PruneList;
use crate::{read_bitmap, save_via_temp_file};

use std::fs::File;
use std::io::{self, BufWriter, Write};

/// Compact (roaring) bitmap representing the set of positions of
/// leaves that are currently unpruned in the MMR.
pub struct LeafSet {
	path: PathBuf,
	bitmap: Bitmap,
	bitmap_bak: Bitmap,
}

impl LeafSet {
	/// Open the remove log file.
	/// The content of the file will be read in memory for fast checking.
	pub fn open<P: AsRef<Path>>(path: P) -> io::Result<LeafSet> {
		let file_path = path.as_ref();
		let bitmap = if file_path.exists() {
			read_bitmap(&file_path)?
		} else {
			Bitmap::create()
		};

		if !bitmap.is_empty() {
			debug!(
				"bitmap {} pos ({} bytes)",
				bitmap.cardinality(),
				bitmap.get_serialized_size_in_bytes(),
			);
		}

		Ok(LeafSet {
			path: file_path.to_path_buf(),
			bitmap_bak: bitmap.clone(),
			bitmap,
		})
	}

	/// Copies a snapshot of the utxo file into the primary utxo file.
	pub fn copy_snapshot<P: AsRef<Path>>(path: P, cp_path: P) -> io::Result<()> {
		let cp_file_path = cp_path.as_ref();

		if !cp_file_path.exists() {
			debug!(
				"leaf_set: rewound leaf file not found: {}",
				cp_file_path.display()
			);
			return Ok(());
		}

		let bitmap = read_bitmap(&cp_file_path)?;
		debug!(
			"leaf_set: copying rewound file {} to {}",
			cp_file_path.display(),
			path.as_ref().display()
		);

		let mut leaf_set = LeafSet {
			path: path.as_ref().to_path_buf(),
			bitmap_bak: bitmap.clone(),
			bitmap,
		};

		leaf_set.flush()?;
		Ok(())
	}

	/// Calculate the set of unpruned leaves
	/// up to and including the cutoff_pos.
	/// Only applicable for the output MMR.
	fn unpruned_pre_cutoff(&self, cutoff_pos: u64, prune_list: &PruneList) -> Bitmap {
		(1..=cutoff_pos)
			.filter(|&x| pmmr::is_leaf(x - 1) && !prune_list.is_pruned(x - 1))
			.map(|x| x as u32)
			.collect()
	}

	/// Calculate the set of pruned positions
	/// up to and including the cutoff_pos.
	/// Uses both the leaf_set and the prune_list to determine prunedness.
	pub fn removed_pre_cutoff(
		&self,
		cutoff_pos: u64,
		rewind_rm_pos: &Bitmap,
		prune_list: &PruneList,
	) -> Bitmap {
		let mut bitmap = self.bitmap.clone();

		// First remove pos from leaf_set that were
		// added after the point we are rewinding to.
		let to_remove = ((cutoff_pos + 1) as u32)..bitmap.maximum().unwrap_or(0);
		bitmap.remove_range_closed(to_remove);

		// Then add back output pos to the leaf_set
		// that were removed.
		bitmap.or_inplace(&rewind_rm_pos);

		// Invert bitmap for the leaf pos and return the resulting bitmap.
		bitmap
			.flip(1..(cutoff_pos + 1))
			.and(&self.unpruned_pre_cutoff(cutoff_pos, prune_list))
	}

	/// Rewinds the leaf_set back to a previous state.
	/// Removes all pos after the cutoff.
	/// Adds back all pos in rewind_rm_pos.
	pub fn rewind(&mut self, cutoff_pos: u64, rewind_rm_pos: &Bitmap) {
		// First remove pos from leaf_set that were
		// added after the point we are rewinding to.
		let to_remove = ((cutoff_pos + 1) as u32)..self.bitmap.maximum().unwrap_or(0);
		self.bitmap.remove_range_closed(to_remove);

		// Then add back output pos to the leaf_set
		// that were removed.
		self.bitmap.or_inplace(&rewind_rm_pos);
	}

	/// Append a new position to the leaf_set.
	pub fn add(&mut self, pos0: u64) {
		self.bitmap.add(1 + pos0 as u32);
	}

	/// Remove the provided position from the leaf_set.
	pub fn remove(&mut self, pos0: u64) {
		self.bitmap.remove(1 + pos0 as u32);
	}

	/// Saves the utxo file tagged with block hash as filename suffix.
	/// Needed during fast-sync as the receiving node cannot rewind
	/// after receiving the txhashset zip file.
	pub fn snapshot(&self, header: &BlockHeader) -> io::Result<()> {
		let mut cp_bitmap = self.bitmap.clone();
		cp_bitmap.run_optimize();

		let cp_path = format!("{}.{}", self.path.to_str().unwrap(), header.hash());
		let mut file = BufWriter::new(File::create(cp_path)?);
		file.write_all(&cp_bitmap.serialize())?;
		file.flush()?;
		Ok(())
	}

	/// Flush the leaf_set to file.
	pub fn flush(&mut self) -> io::Result<()> {
		// First run the optimization step on the bitmap.
		self.bitmap.run_optimize();

		// Write the updated bitmap file to disk.
		save_via_temp_file(&self.path, ".tmp", |file| {
			file.write_all(&self.bitmap.serialize())
		})?;

		// Make sure our backup in memory is up to date.
		self.bitmap_bak = self.bitmap.clone();

		Ok(())
	}

	/// Discard any pending changes.
	pub fn discard(&mut self) {
		self.bitmap = self.bitmap_bak.clone();
	}

	/// Whether the leaf_set includes the provided position.
	pub fn includes(&self, pos0: u64) -> bool {
		self.bitmap.contains(1 + pos0 as u32)
	}

	/// Number of positions stored in the leaf_set.
	pub fn len(&self) -> usize {
		self.bitmap.cardinality() as usize
	}

	/// Number of positions up to index n in the leaf set
	pub fn n_unpruned_leaves_to_index(&self, to_index: u64) -> u64 {
		self.bitmap.range_cardinality(0..to_index)
	}

	/// Is the leaf_set empty.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// Iterator over positionns in the leaf_set (all leaf positions).
	pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
		self.bitmap.iter().map(|x| x as u64)
	}
}
