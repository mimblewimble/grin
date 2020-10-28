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

//! Generation of the various necessary segments requested during PIBD.

use crate::core::core::BlockHeader;
use crate::error::Error;
use crate::txhashset::{self, TxHashSet};

/// TODO - Replace this with SegmentIdentifier from segment PR.
pub struct SegmentIdentifier {
	height: u8,
	idx: u64,
}

/// Segmenter for generating PIBS segments.
pub struct Segmenter<'a> {
	txhashset: &'a TxHashSet,
	header: BlockHeader,
}

impl<'a> Segmenter<'a> {
	/// Create a new segmenter based on the provided txhashset.
	pub fn new(txhashset: &'a TxHashSet, header: BlockHeader) -> Segmenter<'a> {
		Segmenter { txhashset, header }
	}

	/// Create a kernel segment.
	pub fn kernel_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		txhashset::rewindable_kernel_view(&self.txhashset, |view, batch| {
			view.rewind(&self.header)?;

			// now build a segment from the kernel MMR in our view.

			Ok(())
		})?;

		Ok("this will return a segment")
	}

	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	pub fn bitmap_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		unimplemented!()
	}

	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	pub fn output_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		unimplemented!()
	}

	/// Create a rangeproof segment.
	pub fn rangeproof_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		unimplemented!()
	}
}
