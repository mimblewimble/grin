// Copyright 2021 The Grin Developers
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

use std::{sync::Arc, time::Instant};

use crate::core::core::hash::Hash;
use crate::core::core::pmmr::ReadablePMMR;
use crate::core::core::{BlockHeader, OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::error::Error;
use crate::txhashset::{BitmapAccumulator, BitmapChunk, TxHashSet};
use crate::util::secp::pedersen::RangeProof;
use crate::util::RwLock;

/// Segmenter for generating PIBD segments.
#[derive(Clone)]
pub struct Segmenter {
	txhashset: Arc<RwLock<TxHashSet>>,
	bitmap_snapshot: Arc<BitmapAccumulator>,
	header: BlockHeader,
}

impl Segmenter {
	/// Create a new segmenter based on the provided txhashset.
	pub fn new(
		txhashset: Arc<RwLock<TxHashSet>>,
		bitmap_snapshot: Arc<BitmapAccumulator>,
		header: BlockHeader,
	) -> Segmenter {
		Segmenter {
			txhashset,
			bitmap_snapshot,
			header,
		}
	}

	/// Header associated with this segmenter instance.
	/// The bitmap "snapshot" corresponds to rewound state at this header.
	pub fn header(&self) -> &BlockHeader {
		&self.header
	}

	/// Create a kernel segment.
	pub fn kernel_segment(&self, id: SegmentIdentifier) -> Result<Segment<TxKernel>, Error> {
		let now = Instant::now();
		let txhashset = self.txhashset.read();
		let kernel_pmmr = txhashset.kernel_pmmr_at(&self.header);
		let segment = Segment::from_pmmr(id, &kernel_pmmr, false)?;
		debug!(
			"kernel_segment: id: ({}, {}), leaves: {}, hashes: {}, proof hashes: {}, took {}ms",
			segment.id().height,
			segment.id().idx,
			segment.leaf_iter().count(),
			segment.hash_iter().count(),
			segment.proof().size(),
			now.elapsed().as_millis()
		);
		Ok(segment)
	}

	/// The root of the output PMMR based on size from the header.
	fn output_root(&self) -> Result<Hash, Error> {
		let txhashset = self.txhashset.read();
		let pmmr = txhashset.output_pmmr_at(&self.header);
		let root = pmmr.root().map_err(&Error::TxHashSetErr)?;
		Ok(root)
	}

	/// The root of the bitmap snapshot PMMR.
	fn bitmap_root(&self) -> Result<Hash, Error> {
		let pmmr = self.bitmap_snapshot.readonly_pmmr();
		let root = pmmr.root().map_err(&Error::TxHashSetErr)?;
		Ok(root)
	}

	/// Create a utxo bitmap segment based on our bitmap "snapshot" and return it with
	/// the corresponding output root.
	pub fn bitmap_segment(
		&self,
		id: SegmentIdentifier,
	) -> Result<(Segment<BitmapChunk>, Hash), Error> {
		let now = Instant::now();
		let bitmap_pmmr = self.bitmap_snapshot.readonly_pmmr();
		let segment = Segment::from_pmmr(id, &bitmap_pmmr, false)?;
		let output_root = self.output_root()?;
		debug!(
			"bitmap_segment: id: ({}, {}), leaves: {}, hashes: {}, proof hashes: {}, took {}ms",
			segment.id().height,
			segment.id().idx,
			segment.leaf_iter().count(),
			segment.hash_iter().count(),
			segment.proof().size(),
			now.elapsed().as_millis()
		);
		Ok((segment, output_root))
	}

	/// Create an output segment and return it with the corresponding bitmap root.
	pub fn output_segment(
		&self,
		id: SegmentIdentifier,
	) -> Result<(Segment<OutputIdentifier>, Hash), Error> {
		let now = Instant::now();
		let txhashset = self.txhashset.read();
		let output_pmmr = txhashset.output_pmmr_at(&self.header);
		let segment = Segment::from_pmmr(id, &output_pmmr, true)?;
		let bitmap_root = self.bitmap_root()?;
		debug!(
			"output_segment: id: ({}, {}), leaves: {}, hashes: {}, proof hashes: {}, took {}ms",
			segment.id().height,
			segment.id().idx,
			segment.leaf_iter().count(),
			segment.hash_iter().count(),
			segment.proof().size(),
			now.elapsed().as_millis()
		);
		Ok((segment, bitmap_root))
	}

	/// Create a rangeproof segment.
	pub fn rangeproof_segment(&self, id: SegmentIdentifier) -> Result<Segment<RangeProof>, Error> {
		let now = Instant::now();
		let txhashset = self.txhashset.read();
		let pmmr = txhashset.rangeproof_pmmr_at(&self.header);
		let segment = Segment::from_pmmr(id, &pmmr, true)?;
		debug!(
			"rangeproof_segment: id: ({}, {}), leaves: {}, hashes: {}, proof hashes: {}, took {}ms",
			segment.id().height,
			segment.id().idx,
			segment.leaf_iter().count(),
			segment.hash_iter().count(),
			segment.proof().size(),
			now.elapsed().as_millis()
		);
		Ok(segment)
	}
}
