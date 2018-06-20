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

//! The Grin leaf_set implementation.
//! Compact (roaring) bitmap representing the set of leaf positions
//! that exist and are not currently pruned in the MMR.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use croaring::Bitmap;

use core::core::hash::Hashed;
use core::core::pmmr;
use core::core::prune_list::PruneList;
use core::core::BlockHeader;

use util::LOGGER;

/// Compact (roaring) bitmap representing the set of positions of
/// leaves that are currently unpruned in the MMR.
pub struct LeafSet {
	path: String,
	bitmap: Bitmap,
	bitmap_bak: Bitmap,
}

impl LeafSet {
	/// Open the remove log file.
	/// The content of the file will be read in memory for fast checking.
	pub fn open(path: String) -> io::Result<LeafSet> {
		let file_path = Path::new(&path);
		let bitmap = if file_path.exists() {
			let mut bitmap_file = File::open(path.clone())?;
			let mut buffer = vec![];
			bitmap_file.read_to_end(&mut buffer)?;
			Bitmap::deserialize(&buffer)
		} else {
			Bitmap::create()
		};

		Ok(LeafSet {
			path: path.clone(),
			bitmap: bitmap.clone(),
			bitmap_bak: bitmap.clone(),
		})
	}

	/// Copies a snapshot of the utxo file into the primary utxo file.
	pub fn copy_snapshot(path: String, cp_path: String) -> io::Result<()> {
		let cp_file_path = Path::new(&cp_path);

		if !cp_file_path.exists() {
			debug!(LOGGER, "leaf_set: rewound leaf file not found: {}", cp_path);
			return Ok(());
		}

		let mut bitmap_file = File::open(cp_path.clone())?;
		let mut buffer = vec![];
		bitmap_file.read_to_end(&mut buffer)?;
		let bitmap = Bitmap::deserialize(&buffer);

		debug!(
			LOGGER,
			"leaf_set: copying rewound file {} to {}", cp_path, path
		);

		let mut leaf_set = LeafSet {
			path: path.clone(),
			bitmap: bitmap.clone(),
			bitmap_bak: bitmap.clone(),
		};

		leaf_set.flush()?;
		Ok(())
	}

	/// Calculate the set of unpruned leaves
	/// up to and including the cutoff_pos.
	/// Only applicable for the output MMR.
	fn unpruned_pre_cutoff(&self, cutoff_pos: u64, prune_list: &PruneList) -> Bitmap {
		(1..=cutoff_pos)
			.filter(|&x| pmmr::is_leaf(x))
			.filter(|&x| !prune_list.is_pruned(x))
			.map(|x| x as u32)
			.collect()
	}

	/// Calculate the set of pruned positions
	/// up to and including the cutoff_pos.
	/// Uses both the leaf_set and the prune_list to determine prunedness.
	pub fn removed_pre_cutoff(
		&self,
		cutoff_pos: u64,
		rewind_add_pos: &Bitmap,
		rewind_rm_pos: &Bitmap,
		prune_list: &PruneList,
	) -> Bitmap {
		let mut bitmap = self.bitmap.clone();

		// Now "rewind" using the rewind_add_pos and rewind_rm_pos bitmaps passed in.
		bitmap.andnot_inplace(&rewind_add_pos);
		bitmap.or_inplace(&rewind_rm_pos);

		// Invert bitmap for the leaf pos and return the resulting bitmap.
		bitmap
			.flip(1..(cutoff_pos + 1))
			.and(&self.unpruned_pre_cutoff(cutoff_pos, prune_list))
	}

	/// Rewinds the leaf_set back to a previous state.
	pub fn rewind(&mut self, rewind_add_pos: &Bitmap, rewind_rm_pos: &Bitmap) {
		// First remove pos from leaf_set that were
		// added after the point we are rewinding to.
		self.bitmap.andnot_inplace(&rewind_add_pos);
		// Then add back output pos to the leaf_set
		// that were removed.
		self.bitmap.or_inplace(&rewind_rm_pos);
	}

	/// Append a new position to the leaf_set.
	pub fn add(&mut self, pos: u64) {
		self.bitmap.add(pos as u32);
	}

	/// Remove the provided position from the leaf_set.
	pub fn remove(&mut self, pos: u64) {
		self.bitmap.remove(pos as u32);
	}

	/// Saves the utxo file tagged with block hash as filename suffix.
	/// Needed during fast-sync as the receiving node cannot rewind
	/// after receiving the txhashset zip file.
	pub fn snapshot(&self, header: &BlockHeader) -> io::Result<()> {
		let mut cp_bitmap = self.bitmap.clone();
		cp_bitmap.run_optimize();

		let cp_path = format!("{}.{}", self.path, header.hash());
		let mut file = BufWriter::new(File::create(cp_path)?);
		file.write_all(&cp_bitmap.serialize())?;
		file.flush()?;
		Ok(())
	}

	/// Flush the leaf_set to file.
	pub fn flush(&mut self) -> io::Result<()> {
		// First run the optimization step on the bitmap.
		self.bitmap.run_optimize();

		// TODO - consider writing this to disk in a tmp file and then renaming?

		// Write the updated bitmap file to disk.
		{
			let mut file = BufWriter::new(File::create(self.path.clone())?);
			file.write_all(&self.bitmap.serialize())?;
			file.flush()?;
		}

		// Make sure our backup in memory is up to date.
		self.bitmap_bak = self.bitmap.clone();

		Ok(())
	}

	/// Discard any pending changes.
	pub fn discard(&mut self) {
		self.bitmap = self.bitmap_bak.clone();
	}

	/// Whether the leaf_set includes the provided position.
	pub fn includes(&self, pos: u64) -> bool {
		self.bitmap.contains(pos as u32)
	}

	/// Number of positions stored in the leaf_set.
	pub fn len(&self) -> usize {
		self.bitmap.cardinality() as usize
	}

	/// Is the leaf_set empty.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}
