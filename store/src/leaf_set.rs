// Copyright 2020 The Grin Developers
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

use croaring::Bitmap;

use crate::core::core::pmmr;
use crate::{prune_list::PruneList, read_bitmap, save_via_temp_file};

use std::io;
use std::io::Write;
use std::path::Path;

/// Compact (roaring) bitmap representing the set of positions of
/// leaves that are currently unpruned in the MMR.
pub struct LeafSet {}

impl LeafSet {
	/// The content of the file will be read in memory for fast checking.
	pub fn read<P: AsRef<Path>>(path: P) -> io::Result<Bitmap> {
		let file_path = path.as_ref();
		debug!("leaf_set: {:?}", file_path);
		let bitmap = if file_path.exists() {
			read_bitmap(&file_path)?
		} else {
			Bitmap::create()
		};

		if !bitmap.is_empty() {
			debug!(
				"leaf_set: {} pos ({} bytes)",
				bitmap.cardinality(),
				bitmap.get_serialized_size_in_bytes(),
			);
		}

		Ok(bitmap)
	}

	/// Write leaf_set bitmap out to provided file path.
	pub fn write<P: AsRef<Path>>(path: P, bitmap: &Bitmap) -> io::Result<()> {
		let mut bitmap = bitmap.clone();

		// Run the optimization step on the bitmap.
		bitmap.run_optimize();

		// Write the bitmap file to disk.
		save_via_temp_file(path, ".tmp", |file| file.write_all(&bitmap.serialize()))?;

		Ok(())
	}

	/// Calculate the set of unpruned leaves
	/// up to and including the cutoff_pos.
	/// Only applicable for the output MMR.
	fn unpruned_pre_cutoff(cutoff_pos: u64, prune_list: &PruneList) -> Bitmap {
		(1..=cutoff_pos)
			.filter(|&x| pmmr::is_leaf(x) && !prune_list.is_pruned(x))
			.map(|x| x as u32)
			.collect()
	}

	/// Calculate the set of pruned positions
	/// up to and including the cutoff_pos.
	/// Uses both the leaf_set and the prune_list to determine prunedness.
	pub fn removed_pre_cutoff(cutoff_pos: u64, bitmap: &Bitmap, prune_list: &PruneList) -> Bitmap {
		// Invert bitmap for the leaf pos and return the resulting bitmap.
		bitmap
			.flip(1..(cutoff_pos + 1))
			.and(&LeafSet::unpruned_pre_cutoff(cutoff_pos, prune_list))
	}
}
