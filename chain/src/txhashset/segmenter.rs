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
use crate::txhashset::{self, BitmapChunk, PMMRHandle, TxHashSet};
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
	pub fn kernel_segment(&self, id: SegmentIdentifier) -> Result<Segment<TxKernel>, Error> {
		let txhashset = self.txhashset.read();
		txhashset::rewindable_kernel_view(&txhashset, |view, _| {
			// This rewind is fast as we take advantage of our "rewindable kernel view".
			view.rewind(&self.header)?;
			let pmmr = view.readonly_pmmr();
			let segment = Segment::from_pmmr(id, &pmmr, false)?;
			Ok(segment)
		})
	}

	/// Create a utxo bitmap segment.
	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	pub fn bitmap_segment(
		&self,
		id: SegmentIdentifier,
	) -> Result<(Segment<BitmapChunk>, Hash), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;

			// This rewind is relatively expensive but we need to recreate the utxo (bitmap accumulator)
			// for our specified header.
			// We may want to consider taking a "snapshot" of the bitmap accumulator (write to disk?)
			// to allow for fast subsequent reads?
			extension.rewind(&self.header, batch)?;

			let bitmap_pmmr = extension.bitmap_readonly_pmmr();
			let segment = Segment::from_pmmr(id, &bitmap_pmmr, true)?;

			let output_pmmr = extension.output_readonly_pmmr();
			let output_root = output_pmmr
				.root()
				.map_err(|_| ErrorKind::TxHashSetErr("failed to get output root".into()))?;

			Ok((segment, output_root))
		})
	}

	/// Create an output segment.
	/// Note: we need to rewind both the bitmap and the output MMR as we need both roots here.
	pub fn output_segment(
		&self,
		id: SegmentIdentifier,
	) -> Result<(Segment<OutputIdentifier>, Hash), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;

			// This rewind is relatively expensive as we need to rewind spent outputs over multiple blocks.
			// We may want to revisit this.
			// Possible approach would be to rewind this once and cache the root hashes and PMMR sizes.
			// Then we can directly init a ReadonlyPMMR with the correct sizes for subsequent reads.
			extension.rewind(&self.header, batch)?;

			let output_pmmr = extension.output_readonly_pmmr();
			let segment = Segment::from_pmmr(id, &output_pmmr, true)?;

			let bitmap_pmmr = extension.bitmap_readonly_pmmr();
			let bitmap_root = bitmap_pmmr
				.root()
				.map_err(|_| ErrorKind::TxHashSetErr("failed to get bitmap root".into()))?;

			Ok((segment, bitmap_root))
		})
	}

	/// Create a rangeproof segment.
	pub fn rangeproof_segment(&self, id: SegmentIdentifier) -> Result<Segment<RangeProof>, Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		txhashset::extending_readonly(&mut header_pmmr, &mut txhashset, |ext, batch| {
			let extension = &mut ext.extension;

			// This rewind is relatively expensive as we need to rewind (unpend) outputs over multiple blocks.
			// We may want to revisit this.
			// Possible approach would be to rewind this once and cache the root hashes and PMMR sizes.
			// Then we can directly init a ReadonlyPMMR with the correct sizes for subsequent reads.
			extension.rewind(&self.header, batch)?;

			let pmmr = extension.rproof_readonly_pmmr();
			let segment = Segment::from_pmmr(id, &pmmr, false)?;
			Ok(segment)
		})
	}
}
