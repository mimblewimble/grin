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

//! The Grin UTXO Set implementation.
//! Compact (roaring) bitmap representing the set of positions of
//! unspent outputs (UTXO) in the output MMR.

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
/// unspent outputs (UTXO) in the output MMR.
pub struct UtxoSet {
	path: String,
	bitmap: Bitmap,
	bitmap_bak: Bitmap,
}

impl UtxoSet {
	/// Open the remove log file.
	/// The content of the file will be read in memory for fast checking.
	pub fn open(path: String) -> io::Result<UtxoSet> {
		let file_path = Path::new(&path);
		let bitmap = if file_path.exists() {
			let mut bitmap_file = File::open(path.clone())?;
			let mut buffer = vec![];
			bitmap_file.read_to_end(&mut buffer)?;
			Bitmap::deserialize(&buffer)
		} else {
			Bitmap::create()
		};

		Ok(UtxoSet {
			path: path.clone(),
			bitmap: bitmap.clone(),
			bitmap_bak: bitmap.clone(),
		})
	}

	pub fn copy_from(path: String, cp_path: String) -> io::Result<()> {
		let cp_file_path = Path::new(&cp_path);

		if !cp_file_path.exists() {
			debug!(LOGGER, "utxo_set: rewound utxo file not found: {}", cp_path);
			return Ok(());
		}

		let mut bitmap_file = File::open(cp_path.clone())?;
		let mut buffer = vec![];
		bitmap_file.read_to_end(&mut buffer)?;
		let bitmap = Bitmap::deserialize(&buffer);

		debug!(
			LOGGER,
			"utxo_set: copying rewound file {} to {}", cp_path, path
		);

		let mut utxo_set = UtxoSet {
			path: path.clone(),
			bitmap: bitmap.clone(),
			bitmap_bak: bitmap.clone(),
		};

		utxo_set.flush()?;
		Ok(())
	}

	/// Calculate the set of positions of all unspent outputs
	/// up to and including the cutoff_pos.
	/// Returns these positions as a bitmap.
	/// Only applicable for the output MMR.
	pub fn utxo_lte_pos(&self, cutoff_pos: u64) -> Bitmap {
		let bitmask: Bitmap = (1..=cutoff_pos).map(|x| x as u32).collect();
		self.bitmap.and(&bitmask)
	}

	/// Calculate the set of unpruned leaves
	/// up to and including the cutoff_pos.
	/// Only applicable for the output MMR.
	fn unpruned_leaves_lte_pos(&self, cutoff_pos: u64, prune_list: &PruneList) -> Bitmap {
		(1..=cutoff_pos)
			.filter(|&x| pmmr::is_leaf(x))
			.filter(|&x| !prune_list.is_pruned(x))
			.map(|x| x as u32)
			.collect()
	}

	/// Calculate the set of spent positions
	/// up to and including the cutoff_pos.
	/// Takes the prune_list into account when
	/// calculating these spent positions (anything pruned is spent).
	/// Only applicable for the output MMR.
	pub fn spent_lte_pos(&self, cutoff_pos: u64, prune_list: &PruneList) -> Bitmap {
		self.utxo_lte_pos(cutoff_pos)
			.flip(1..(cutoff_pos + 1))
			.and(&self.unpruned_leaves_lte_pos(cutoff_pos, prune_list))
	}

	/// Rewinds the UTXO set back to a previous state.
	pub fn rewind(&mut self, rewind_output_pos: &Bitmap, rewind_spent_pos: &Bitmap) {
		// First remove output pos from UTXO set that were
		// added after the point we are rewinding to.
		self.bitmap.andnot_inplace(&rewind_output_pos);
		// Then add back output pos to the UTXO set that were
		// spent after the point we are rewinding to.
		self.bitmap.or_inplace(&rewind_spent_pos);
	}

	/// Append a new position to the UTXO set.
	pub fn add(&mut self, pos: u64) {
		self.bitmap.add(pos as u32);
	}

	/// Remove the provided position from the UTXO set.
	pub fn remove(&mut self, pos: u64) {
		self.bitmap.remove(pos as u32);
	}

	pub fn save_copy(&self, header: &BlockHeader) -> io::Result<()> {
		let mut cp_bitmap = self.bitmap.clone();
		cp_bitmap.run_optimize();

		let cp_path = format!("{}.{}", self.path, header.hash());
		let mut file = BufWriter::new(File::create(cp_path)?);
		file.write_all(&cp_bitmap.serialize())?;
		file.flush()?;
		Ok(())
	}

	/// Flush the UTXO set to file.
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

	/// Whether the UTXO set includes the provided position.
	pub fn includes(&self, pos: u64) -> bool {
		self.bitmap.contains(pos as u32)
	}

	/// Number of positions stored in the UTXO set.
	pub fn len(&self) -> usize {
		self.bitmap.cardinality() as usize
	}
}
