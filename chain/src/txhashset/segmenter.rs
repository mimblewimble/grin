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

use std::{marker, sync::Arc};

use crate::core::core::hash::Hash;
use crate::core::core::pmmr::{Backend, ReadablePMMR, ReadonlyPMMR};
use crate::core::core::{BlockHeader, OutputIdentifier, TxKernel};
use crate::core::ser::{PMMRable, Readable, Writeable};
use crate::error::{Error, ErrorKind};
use crate::txhashset::{BitmapAccumulator, BitmapChunk, TxHashSet};
use crate::util::secp::pedersen::RangeProof;
use crate::util::RwLock;

/// TODO - Replace this with SegmentIdentifier from segment PR.
pub struct SegmentIdentifier {
	height: u8,
	idx: u64,
}

/// TODO - Replace this the real Segment
pub struct Segment<T> {
	id: SegmentIdentifier,
	_marker: marker::PhantomData<T>,
}

/// TODO - Replace this the real Segment
impl<T> Segment<T>
where
	T: std::fmt::Debug + Readable + Writeable,
{
	/// Generate a segment from a PMMR
	pub fn from_pmmr<U, B>(
		segment_id: SegmentIdentifier,
		pmmr: &ReadonlyPMMR<'_, U, B>,
		prunable: bool,
	) -> Result<Self, Error>
	where
		U: PMMRable<E = T>,
		B: Backend<U>,
	{
		Ok(Segment {
			id: segment_id,
			_marker: marker::PhantomData,
		})
	}
}

/// Segmenter for generating PIBD segments.
#[derive(Clone)]
pub struct Segmenter {
	txhashset: Arc<RwLock<TxHashSet>>,
	bitmap_snapshot: BitmapAccumulator,
	header: BlockHeader,
}

impl Segmenter {
	/// Create a new segmenter based on the provided txhashset.
	pub fn new(
		txhashset: Arc<RwLock<TxHashSet>>,
		bitmap_snapshot: BitmapAccumulator,
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
		let txhashset = self.txhashset.read();
		let kernel_pmmr = txhashset.kernel_pmmr_at(&self.header);
		let segment = Segment::from_pmmr(id, &kernel_pmmr, false)?;
		Ok(segment)
	}

	/// The root of the output PMMR based on size from the header.
	fn output_root(&self) -> Result<Hash, Error> {
		let txhashset = self.txhashset.read();
		let pmmr = txhashset.output_pmmr_at(&self.header);
		let root = pmmr.root().map_err(&ErrorKind::TxHashSetErr)?;
		Ok(root)
	}

	/// The root of the bitmap snapshot PMMR.
	fn bitmap_root(&self) -> Result<Hash, Error> {
		let pmmr = self.bitmap_snapshot.readonly_pmmr();
		let root = pmmr.root().map_err(&ErrorKind::TxHashSetErr)?;
		Ok(root)
	}

	/// Create a utxo bitmap segment based on our bitmap "snapshot" and return it with
	/// the corresponding output root.
	pub fn bitmap_segment(
		&self,
		id: SegmentIdentifier,
	) -> Result<(Segment<BitmapChunk>, Hash), Error> {
		let bitmap_pmmr = self.bitmap_snapshot.readonly_pmmr();
		let segment = Segment::from_pmmr(id, &bitmap_pmmr, false)?;
		let output_root = self.output_root()?;
		Ok((segment, output_root))
	}

	/// Create an output segment and return it with the corresponding bitmap root.
	pub fn output_segment(
		&self,
		id: SegmentIdentifier,
	) -> Result<(Segment<OutputIdentifier>, Hash), Error> {
		let txhashset = self.txhashset.read();
		let output_pmmr = txhashset.output_pmmr_at(&self.header);
		let segment = Segment::from_pmmr(id, &output_pmmr, true)?;
		let bitmap_root = self.bitmap_root()?;
		Ok((segment, bitmap_root))
	}

	/// Create a rangeproof segment.
	pub fn rangeproof_segment(&self, id: SegmentIdentifier) -> Result<Segment<RangeProof>, Error> {
		let txhashset = self.txhashset.read();
		let pmmr = txhashset.rangeproof_pmmr_at(&self.header);
		let segment = Segment::from_pmmr(id, &pmmr, true)?;
		Ok(segment)
	}
}
