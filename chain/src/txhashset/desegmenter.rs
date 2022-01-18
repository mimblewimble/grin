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

use std::sync::Arc;

use crate::core::core::hash::Hash;
use crate::core::core::{pmmr, pmmr::ReadablePMMR};
use crate::core::core::{BlockHeader, OutputIdentifier, Segment, SegmentIdentifier, TxKernel};
use crate::error::Error;
use crate::txhashset::{BitmapAccumulator, BitmapChunk, TxHashSet};
use crate::util::secp::pedersen::RangeProof;
use crate::util::RwLock;

use crate::store;
use crate::txhashset;

use croaring::Bitmap;

/// States that the desegmenter can be in, to keep track of what
/// parts are needed next in the proces
#[derive(Clone)]
pub enum DesegmenterState {
	/// Uninitialised state
	Uninitialised,
	/// Needs Output set bitmap. Ironically also contains a bitmap representing
	/// what segments of bitmap are still needed
	NeedsOutputSetBitmap {
		/// Total required number of segments,
		/// When we have this we can finalize
		required_segment_count: usize,
	},
}

/// Desegmenter for rebuilding a txhashset from PIBD segments
#[derive(Clone)]
pub struct Desegmenter {
	txhashset: Arc<RwLock<TxHashSet>>,
	header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
	archive_header: BlockHeader,
	store: Arc<store::ChainStore>,

	state: DesegmenterState,

	default_segment_height: u8,

	bitmap_accumulator: BitmapAccumulator,
	_bitmap_segments: Vec<Segment<BitmapChunk>>,
	_output_segments: Vec<Segment<OutputIdentifier>>,
	_rangeproof_segments: Vec<Segment<RangeProof>>,
	_kernel_segments: Vec<Segment<TxKernel>>,

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
			state: DesegmenterState::Uninitialised,
			default_segment_height: 9,
			_bitmap_segments: vec![],
			_output_segments: vec![],
			_rangeproof_segments: vec![],
			_kernel_segments: vec![],

			bitmap_mmr_leaf_count: 0,
			bitmap_mmr_size: 0,

			bitmap_cache: None,
		};
		retval.calc_bitmap_mmr_sizes();
		retval.state = DesegmenterState::NeedsOutputSetBitmap {
			required_segment_count: SegmentIdentifier::count_segments_required(
				retval.bitmap_mmr_size,
				retval.default_segment_height,
			),
		};
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

	/// Return list of the next preferred segments the desegmenter needs based on
	/// the current real state of the underlying elements
	pub fn next_desired_segments(&self, max_elements: usize) -> Vec<SegmentIdentifier> {
		let mut return_vec = vec![];

		// First check for required bitmap elements
		if self.bitmap_cache.is_none() {
			debug!("Desegmenter needs bitmap segments");
			// Get current size of bitmap MMR
			let local_pmmr_size = self.bitmap_accumulator.readonly_pmmr().unpruned_size();
			debug!("Local Bitmap PMMR Size is: {}", local_pmmr_size);
			// Get iterator over expected bitmap elements
			let mut identifier_iter = SegmentIdentifier::traversal_iter(
				self.bitmap_mmr_size,
				self.default_segment_height,
			);
			debug!("Expected bitmap MMR size is: {}", self.bitmap_mmr_size);
			// Advance iterator to next expected segment
			while let Some(id) = identifier_iter.next() {
				debug!(
					"ID segment pos range: {:?}",
					id.segment_pos_range(self.bitmap_mmr_size)
				);
				if id.segment_pos_range(self.bitmap_mmr_size).1 > local_pmmr_size {
					return_vec.push(id);
					if return_vec.len() >= max_elements {
						return return_vec;
					}
				}
			}
		}
		return_vec
	}

	/// 'Finalize' the bitmap accumulator, storing an in-memory copy of the bitmap for
	/// use in further validation and setting the accumulator on the underlying txhashset
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

		// Set the txhashset's bitmap accumulator
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, _batch| {
				let extension = &mut ext.extension;
				// TODO: Unwrap
				extension.set_bitmap_accumulator(self.bitmap_accumulator.clone());
				Ok(())
			},
		)?;
		Ok(())
	}

	// Calculate and store number of leaves and positions in the bitmap mmr given the number of
	// outputs specified in the header. Should be called whenever the header changes
	fn calc_bitmap_mmr_sizes(&mut self) {
		// Number of leaves (BitmapChunks)
		self.bitmap_mmr_leaf_count =
			(pmmr::n_leaves(self.archive_header.output_mmr_size) + 1023) / 1024;
		debug!(
			"pibd_desgmenter - expected number of leaves in bitmap MMR: {}",
			self.bitmap_mmr_leaf_count
		);
		// Total size of Bitmap PMMR
		self.bitmap_mmr_size =
			1 + pmmr::peaks(pmmr::insertion_to_pmmr_index(self.bitmap_mmr_leaf_count))
				.last()
				.unwrap_or(
					&(pmmr::peaks(pmmr::insertion_to_pmmr_index(
						self.bitmap_mmr_leaf_count - 1,
					))
					.last()
					.unwrap()),
				)
				.clone();

		debug!(
			"pibd_desgmenter - expected size of bitmap MMR: {}",
			self.bitmap_mmr_size
		);
	}

	/// Adds and validates a bitmap chunk
	/// TODO: Still experimenting, this expects chunks received to be in order
	pub fn add_bitmap_segment(
		&mut self,
		segment: Segment<BitmapChunk>,
		output_root_hash: Hash,
	) -> Result<(), Error> {
		debug!("pibd_desegmenter: add bitmap segment");
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
	/// TODO: Still experimenting, expects chunks received to be in order
	pub fn add_output_segment(
		&self,
		segment: Segment<OutputIdentifier>,
		_bitmap_root: Option<Hash>,
	) -> Result<(), Error> {
		debug!("pibd_desegmenter: add output segment");
		// TODO: check bitmap root matches what we already have
		segment.validate_with(
			self.archive_header.output_mmr_size, // Last MMR pos at the height being validated
			self.bitmap_cache.as_ref(),
			self.archive_header.output_root, // Output root we're checking for
			self.archive_header.output_mmr_size,
			self.bitmap_accumulator.root(), // Other root
			false,
		)?;
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, _batch| {
				let extension = &mut ext.extension;
				extension.apply_output_segment(segment)?;
				Ok(())
			},
		)?;
		Ok(())
	}

	/// Adds a Rangeproof segment
	/// TODO: Still experimenting, expects chunks received to be in order
	pub fn add_rangeproof_segment(&self, segment: Segment<RangeProof>) -> Result<(), Error> {
		debug!("pibd_desegmenter: add rangeproof segment");
		segment.validate(
			self.archive_header.output_mmr_size, // Last MMR pos at the height being validated
			self.bitmap_cache.as_ref(),
			self.archive_header.range_proof_root, // Range proof root we're checking for
		)?;
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, _batch| {
				let extension = &mut ext.extension;
				extension.apply_rangeproof_segment(segment)?;
				Ok(())
			},
		)?;
		Ok(())
	}

	/// Adds a Kernel segment
	/// TODO: Still experimenting, expects chunks received to be in order
	pub fn add_kernel_segment(&self, segment: Segment<TxKernel>) -> Result<(), Error> {
		debug!("pibd_desegmenter: add kernel segment");
		segment.validate(
			self.archive_header.kernel_mmr_size, // Last MMR pos at the height being validated
			None,
			self.archive_header.kernel_root, // Kernel root we're checking for
		)?;
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut batch = self.store.batch()?;
		txhashset::extending(
			&mut header_pmmr,
			&mut txhashset,
			&mut batch,
			|ext, _batch| {
				let extension = &mut ext.extension;
				extension.apply_kernel_segment(segment)?;
				Ok(())
			},
		)?;
		Ok(())
	}
}
