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

//! Common storage-related types
use memmap;

use std::cmp;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::io::{self, BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

use croaring::Bitmap;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
use libc::{ftruncate as ftruncate64, off_t as off64_t};
#[cfg(any(target_os = "linux"))]
use libc::{ftruncate64, off64_t};

use core::core::pmmr;
use core::core::prune_list::PruneList;
use core::ser;

/// ~Log file~ fully cached in memory containing all positions that should be
/// eventually removed from the MMR append-only data file. Allows quick
/// checking of whether a piece of data has been marked for deletion. ~When the
/// log becomes too long, the MMR backend will actually remove chunks from the
/// MMR data file and truncate the remove log.~
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

	pub fn utxo_lte_pos(&self, cutoff_pos: u64) -> Bitmap {
		let bitmask: Bitmap = (1..=cutoff_pos).map(|x| x as u32).collect();
		self.bitmap.and(&bitmask)
	}

	// TODO - Probably a more efficient way of doing this.
	// TODO - Should be able to translate prune list into bitmap of "unpruned" leaf
	// pos.
	fn unpruned_leaves_lte_pos(&self, cutoff_pos: u64, prune_list: &PruneList) -> Bitmap {
		(1..=cutoff_pos)
			.filter(|&x| pmmr::is_leaf(x))
			.filter(|&x| !prune_list.is_pruned(x))
			.map(|x| x as u32)
			.collect()
	}

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
