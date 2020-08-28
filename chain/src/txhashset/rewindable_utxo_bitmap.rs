// Copyright 2020 The Grin Developers
//
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

//! Lightweight readonly view into utxo bitmap for convenience.

use crate::core::core::hash::Hashed;
use crate::core::core::BlockHeader;
use crate::{error::Error, store::Batch, types::CommitPos, Tip};
use croaring::Bitmap;

/// Rewindable view of the utxo bitmap.
pub struct RewindableUtxoBitmap {
	bitmap: Bitmap,
	head: Tip,
}

impl RewindableUtxoBitmap {
	/// Build a new rewindable utxo bitmap.
	pub fn new(bitmap: &Bitmap, head: Tip) -> RewindableUtxoBitmap {
		RewindableUtxoBitmap {
			bitmap: bitmap.clone(),
			head,
		}
	}

	/// Rewind the utxo bitmap to the provided block.
	pub fn rewind(&mut self, header: &BlockHeader, batch: &Batch<'_>) -> Result<Bitmap, Error> {
		let mut current = batch.head_header()?;
		while header.height < current.height {
			self.rewind_single_block(&current, batch)?;
			current = batch.get_previous_header(&current)?;
		}
		self.head = Tip::from_header(header);
		Ok(self.bitmap.clone())
	}

	/// Rewind the utxo bitmap by a single block.
	pub fn rewind_single_block(
		&mut self,
		header: &BlockHeader,
		batch: &Batch<'_>,
	) -> Result<Vec<CommitPos>, Error> {
		let prev = batch.get_previous_header(&header)?;
		// The spent index allows us to conveniently "unspend" everything in a block.
		let spent = batch.get_spent_index(&header.hash())?;
		let spent_bitmap: Bitmap = spent.iter().map(|x| x.pos as u32).collect();

		// Remove everything added to the bitmap by this block.
		self.bitmap.remove_range_closed(
			((prev.output_mmr_size + 1) as u32)..self.bitmap.maximum().unwrap_or(0),
		);

		// Add back everything spent by this block.
		self.bitmap.or_inplace(&spent_bitmap);
		Ok(spent)
	}
}

impl From<RewindableUtxoBitmap> for Bitmap {
	fn from(utxo: RewindableUtxoBitmap) -> Self {
		utxo.bitmap
	}
}
