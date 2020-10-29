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

use crate::core::core::hash::Hash;
use crate::core::core::pmmr::ReadablePMMR;
use crate::core::core::BlockHeader;
use crate::error::{Error, ErrorKind};
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
			// This rewind is fast as we take advantage of our "rewindable kernel view".
			view.rewind(&self.header)?;

			// TODO - Generate segment from our rewound kernel view.
			let pmmr = view.readonly_pmmr();
			// let segment = Segment::from_pmmr(id, &pmmr, false)?;

			Ok(())
		})?;

		Ok("this will return a segment")
	}

	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	/// TODO - Return a segment and the corresponding output root.
	pub fn bitmap_segment(&self, _id: SegmentIdentifier) -> Result<(&str, Hash), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;

			// This rewind is relatively expensive but we need to recreate the utxo (bitmap accumulator)
			// for our specified header.
			// We may want to consider taking a "snapshot" of the bitmap accumulator (write to disk?)
			// to allow for fast subsequent reads?
			extension.rewind(&self.header, batch)?;

			// TODO - Generate segment from our rewound extension.
			let bitmap_pmmr = extension.bitmap_readonly_pmmr();
			// let segment = Segment::from_pmmr(id, &bitmap_pmmr, true)?;

			let output_pmmr = extension.output_readonly_pmmr();
			let output_root = output_pmmr
				.root()
				.map_err(|_| ErrorKind::TxHashSetErr("failed to get output root".into()))?;

			Ok((
				"this will return a bitmap segment and corresponding output root",
				output_root,
			))
		})
	}

	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	/// TODO - Return a segment and the corresponding bitmap root.
	pub fn output_segment(&self, _id: SegmentIdentifier) -> Result<(&str, Hash), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;

			// This rewind is relatively expensive as we need to rewind spent outputs over multiple blocks.
			// We may want to revisit this.
			// Possible approach would be to rewind this once and cache the root hashes and PMMR sizes.
			// Then we can directly init a ReadonlyPMMR with the correct sizes for subsequent reads.
			extension.rewind(&self.header, batch)?;

			// TODO - Generate segment from our rewound extension.
			let output_pmmr = extension.output_readonly_pmmr();
			// let segment = Segment::from_pmmr(id, &output_pmmr, true)?;

			let bitmap_pmmr = extension.bitmap_readonly_pmmr();
			let bitmap_root = bitmap_pmmr
				.root()
				.map_err(|_| ErrorKind::TxHashSetErr("failed to get bitmap root".into()))?;

			Ok((
				"this will return an output segment and corresponding bitmap root",
				bitmap_root,
			))
		})
	}

	/// Create a rangeproof segment.
	pub fn rangeproof_segment(&self, _id: SegmentIdentifier) -> Result<&str, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;

			// This rewind is relatively expensive as we need to rewind (unpend) outputs over multiple blocks.
			// We may want to revisit this.
			// Possible approach would be to rewind this once and cache the root hashes and PMMR sizes.
			// Then we can directly init a ReadonlyPMMR with the correct sizes for subsequent reads.
			extension.rewind(&self.header, batch)?;

			// TODO - Generate segment from our rewound extension.
			let pmmr = extension.rproof_readonly_pmmr();
			// let segment = Segment::from_pmmr(id, &pmmr, false)?;

			Ok("this will return a segment")
		})
	}
}
