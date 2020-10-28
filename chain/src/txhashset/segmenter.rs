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

use std::sync::Arc;

use crate::core::core::BlockHeader;
use crate::error::Error;
use crate::txhashset::{self, PMMRHandle, TxHashSet};
use crate::util::RwLock;

/// TODO - Replace this with SegmentIdentifier from segment PR.
pub struct SegmentIdentifier {
	height: u8,
	idx: u64,
}

/// Segmenter for generating PIBS segments.
pub struct Segmenter {
	header_pmmr: Arc<RwLock<PMMRHandle<BlockHeader>>>,
	txhashset: Arc<RwLock<TxHashSet>>,
	header: BlockHeader,
}

impl Segmenter {
	/// Create a new segmenter based on the provided txhashset.
	pub fn new(
		header_pmmr: Arc<RwLock<PMMRHandle<BlockHeader>>>,
		txhashset: Arc<RwLock<TxHashSet>>,
		header: BlockHeader,
	) -> Segmenter {
		Segmenter {
			header_pmmr,
			txhashset,
			header,
		}
	}

	/// Create a kernel segment.
	/// We use a lightweight "rewindable kernel view" here as we do not need to worry about pruning.
	pub fn kernel_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		let txhashset = self.txhashset.read();
		txhashset::rewindable_kernel_view(&txhashset, |view, _| {
			view.rewind(&self.header)?;

			// TODO - Generate segment from our rewound kernel view.
			// let segment = Segment::from_pmmr(id, &view.readonly_pmmr(), false)?;

			Ok(())
		})?;

		Ok("this will return a segment")
	}

	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	/// TODO - Return a segment and the corresponding output root.
	pub fn bitmap_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		unimplemented!()
	}

	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	/// TODO - Return a segment and the corresponding bitmap root.
	pub fn output_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		unimplemented!()
	}

	/// Create a rangeproof segment.
	pub fn rangeproof_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;
			extension.rewind(&self.header, batch)?;

			// TODO - Generate segment from our rewound extension.
			// TODO - We need a readonly version of the rangeproof PMMR here.
			// let segment = Segment::from_pmmr(id, &view.readonly_pmmr(), false)?;

			Ok("this will return a segment")
		})
	}
}
