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

//! Manages the reconsitution of a txhashset from segments produced by the
//! segmenter

use std::{sync::Arc, time::Instant};

use crate::core::core::hash::Hash;
use crate::core::core::pmmr;
use crate::core::core::pmmr::ReadablePMMR;
use crate::core::core::{BlockHeader, OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::error::{Error, ErrorKind};
use crate::txhashset::{BitmapAccumulator, BitmapChunk, TxHashSet};
use crate::util::secp::pedersen::RangeProof;
use crate::util::RwLock;

use crate::store;
use crate::txhashset;

use croaring::Bitmap;

/// Desegmenter for rebuilding a txhashset from PIBD segments
#[derive(Clone)]
pub struct Desegmenter {
	txhashset: Arc<RwLock<TxHashSet>>,
	header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
	archive_header: BlockHeader,
	store: Arc<store::ChainStore>,

	bitmap_accumulator: BitmapAccumulator,
	bitmap_segments: Vec<Segment<BitmapChunk>>,
	output_segments: Vec<Segment<OutputIdentifier>>,
	rangeproof_segments: Vec<Segment<RangeProof>>,
	kernel_segments: Vec<Segment<TxKernel>>,

	bitmap_mmr_leaf_count: u64,
	bitmap_mmr_size: u64,
	// In-memory 'raw' bitmap corresponding to contents of bitmap accumulator
	bitmap_cache: Option<Bitmap>,
}

impl Desegmenter {
	/// Create a new segmenter based on the provided txhashset and the specified block header
	pub fn new(
		txhashset: Arc<RwLock<TxHashSet>>,
		header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
		archive_header: BlockHeader,
		store: Arc<store::ChainStore>,
	) -> Desegmenter {
		let mut retval = Desegmenter {
			txhashset,
			header_pmmr,
			archive_header,
			store,
			bitmap_accumulator: BitmapAccumulator::new(),
			bitmap_segments: vec![],
			output_segments: vec![],
			rangeproof_segments: vec![],
			kernel_segments: vec![],

			bitmap_mmr_leaf_count: 0,
			bitmap_mmr_size: 0,

			bitmap_cache: None,
		};
		retval.calc_bitmap_mmr_sizes();
		retval
	}

	/// Return reference to the header used for validation
	pub fn header(&self) -> &BlockHeader {
		&self.archive_header
	}
	/// Return size of bitmap mmr
	pub fn expected_bitmap_mmr_size(&self) -> u64 {
		self.bitmap_mmr_size
	}

	/// 'Finalize' the bitmap accumulator, storing an in-memory copy of the bitmap for
	/// use in further validation
	/// TODO: Could be called automatically when we have the calculated number of
	/// required segments for the archive header
	/// TODO: Accumulator will likely need to be stored locally to deal with server
	/// being shut down and restarted
	pub fn finalize_bitmap(&mut self) -> Result<(), Error> {
		debug!(
			"pibd_desgmenter: caching bitmap - accumulator root: {}",
			self.bitmap_accumulator.root()
		);
		self.bitmap_cache = Some(self.bitmap_accumulator.as_bitmap()?);
		Ok(())
	}

	// Calculate and store number of leaves and positions in the bitmap mmr given the number of
	// outputs specified in the header. Should be called whenever the header changes
	fn calc_bitmap_mmr_sizes(&mut self) {
		// Number of leaves (BitmapChunks)
		self.bitmap_mmr_leaf_count =
			(pmmr::n_leaves(self.archive_header.output_mmr_size) as f64 / 1024f64).ceil() as u64;
		debug!(
			"pibd_desgmenter - expected number of leaves in bitmap MMR: {}",
			self.bitmap_mmr_leaf_count
		);
		// Total size of Bitmap PMMR
		self.bitmap_mmr_size = pmmr::peaks(self.bitmap_mmr_leaf_count + 1)
			.last()
			.unwrap_or(&pmmr::insertion_to_pmmr_index(self.bitmap_mmr_leaf_count))
			.clone();
		debug!(
			"pibd_desgmenter - expected size of bitmap MMR: {}",
			self.bitmap_mmr_size
		);
	}

	/// Adds and validates a bitmap chunk
	/// NB: Still experimenting, this expects chunks received to be in order
	pub fn add_bitmap_segment(
		&mut self,
		segment: Segment<BitmapChunk>,
		output_root_hash: Hash,
	) -> Result<(), Error> {
		segment.validate_with(
			self.bitmap_mmr_size, // Last MMR pos at the height being validated, in this case of the bitmap root
			None,
			self.archive_header.output_root, // Output root we're checking for
			self.archive_header.output_mmr_size,
			output_root_hash, // Other root
			true,
		)?;
		// All okay, add leaves to bitmap accumulator
		let (_sid, _hash_pos, _hashes, _leaf_pos, leaf_data, _proof) = segment.parts();
		for chunk in leaf_data.into_iter() {
			self.bitmap_accumulator.append_chunk(chunk)?;
		}
		Ok(())
	}

	/// Adds a output segment
	pub fn add_output_segment(&self, segment: Segment<OutputIdentifier>) -> Result<(), Error> {
		debug!("pibd_desegmenter write: add bitmap segment");
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, batch| {
				let extension = &mut ext.extension;

				Ok(())
			},
		)?;
		Ok(())
	}
}
