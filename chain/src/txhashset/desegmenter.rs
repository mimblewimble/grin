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
use crate::core::core::{
	BlockHeader, OutputIdentifier, Segment, SegmentIdentifier, SegmentType, SegmentTypeIdentifier,
	TxKernel,
};
use crate::error::Error;
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

	default_bitmap_segment_height: u8,
	default_output_segment_height: u8,
	default_rangeproof_segment_height: u8,

	bitmap_accumulator: BitmapAccumulator,
	bitmap_segment_cache: Vec<Segment<BitmapChunk>>,
	output_segment_cache: Vec<Segment<OutputIdentifier>>,
	rangeproof_segment_cache: Vec<Segment<RangeProof>>,
	_kernel_segments: Vec<Segment<TxKernel>>,

	bitmap_mmr_leaf_count: u64,
	bitmap_mmr_size: u64,
	/// In-memory 'raw' bitmap corresponding to contents of bitmap accumulator
	bitmap_cache: Option<Bitmap>,

	/// Flag indicating there are no more segments to request
	all_segments_complete: bool,
}

impl Desegmenter {
	/// Create a new segmenter based on the provided txhashset and the specified block header
	pub fn new(
		txhashset: Arc<RwLock<TxHashSet>>,
		header_pmmr: Arc<RwLock<txhashset::PMMRHandle<BlockHeader>>>,
		archive_header: BlockHeader,
		store: Arc<store::ChainStore>,
	) -> Desegmenter {
		trace!("Creating new desegmenter");
		let mut retval = Desegmenter {
			txhashset,
			header_pmmr,
			archive_header,
			store,
			bitmap_accumulator: BitmapAccumulator::new(),
			default_bitmap_segment_height: 9,
			default_output_segment_height: 11,
			default_rangeproof_segment_height: 7,
			bitmap_segment_cache: vec![],
			output_segment_cache: vec![],
			rangeproof_segment_cache: vec![],
			_kernel_segments: vec![],

			bitmap_mmr_leaf_count: 0,
			bitmap_mmr_size: 0,

			bitmap_cache: None,

			all_segments_complete: false,
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

	/// Whether we have all the segments we need
	pub fn is_complete(&self) -> bool {
		self.all_segments_complete
	}

	/// Apply next set of segments that are ready to be appended to their respective trees,
	/// and kick off any validations that can happen. TODO: figure out where and how
	/// this should be called considering any thread blocking implications
	pub fn apply_next_segments(&mut self) -> Result<(), Error> {
		let next_bmp_idx = self.next_required_bitmap_segment_index();
		if let Some(bmp_idx) = next_bmp_idx {
			if let Some((idx, _seg)) = self
				.bitmap_segment_cache
				.iter()
				.enumerate()
				.find(|s| s.1.identifier().idx == bmp_idx)
			{
				self.apply_bitmap_segment(idx)?;
			}
		} else {
			// Check if we need to finalize bitmap
			if self.bitmap_cache == None {
				// Should have all the pieces now, finalize the bitmap cache
				self.finalize_bitmap()?;
			}
			// Check if we can apply the next output segment
			if let Some(next_output_idx) = self.next_required_output_segment_index() {
				trace!("Next output index to apply: {}", next_output_idx);
				if let Some((idx, _seg)) = self
					.output_segment_cache
					.iter()
					.enumerate()
					.find(|s| s.1.identifier().idx == next_output_idx)
				{
					self.apply_output_segment(idx)?;
				}
			}
			// Check if we can apply the next rangeproof segment
			if let Some(next_rangeproof_idx) = self.next_required_rangeproof_segment_index() {
				trace!("Next rangeproof index to apply: {}", next_rangeproof_idx);
				if let Some((idx, _seg)) = self
					.rangeproof_segment_cache
					.iter()
					.enumerate()
					.find(|s| s.1.identifier().idx == next_rangeproof_idx)
				{
					self.apply_rangeproof_segment(idx)?;
				}
			}
			// TODO: Kernel
		}
		Ok(())
	}

	/// Return list of the next preferred segments the desegmenter needs based on
	/// the current real state of the underlying elements
	pub fn next_desired_segments(&mut self, max_elements: usize) -> Vec<SegmentTypeIdentifier> {
		let mut return_vec = vec![];
		// First check for required bitmap elements
		if self.bitmap_cache.is_none() {
			// Get current size of bitmap MMR
			let local_pmmr_size = self.bitmap_accumulator.readonly_pmmr().unpruned_size();
			// Get iterator over expected bitmap elements
			let mut identifier_iter = SegmentIdentifier::traversal_iter(
				self.bitmap_mmr_size,
				self.default_bitmap_segment_height,
			);
			// Advance iterator to next expected segment
			while let Some(id) = identifier_iter.next() {
				if id.segment_pos_range(self.bitmap_mmr_size).1 > local_pmmr_size {
					if !self.has_bitmap_segment_with_id(id) {
						return_vec.push(SegmentTypeIdentifier::new(SegmentType::Bitmap, id));
						if return_vec.len() >= max_elements {
							return return_vec;
						}
					}
				}
			}
		} else {
			// We have all required bitmap segments and have recreated our local
			// bitmap, now continue with other segments
			// TODO: Outputs only for now, just for testing. we'll want to evenly spread
			// requests among the 3 PMMRs
			let local_output_mmr_size;
			let mut _local_kernel_mmr_size;
			let local_rangeproof_mmr_size;
			{
				let txhashset = self.txhashset.read();
				local_output_mmr_size = txhashset.output_mmr_size();
				_local_kernel_mmr_size = txhashset.kernel_mmr_size();
				local_rangeproof_mmr_size = txhashset.rangeproof_mmr_size();
			}
			// TODO: Fix, alternative approach, this is very inefficient
			let mut output_identifier_iter = SegmentIdentifier::traversal_iter(
				self.archive_header.output_mmr_size,
				self.default_output_segment_height,
			);

			while let Some(output_id) = output_identifier_iter.next() {
				// Advance output iterator to next needed position
				if output_id
					.segment_pos_range(self.archive_header.output_mmr_size)
					.1 <= local_output_mmr_size
				{
					continue;
				}
				// Break if we're full
				if return_vec.len() > max_elements {
					break;
				}

				if !self.has_output_segment_with_id(output_id) {
					return_vec.push(SegmentTypeIdentifier::new(SegmentType::Output, output_id));
					// Let other trees have a chance to put in a segment request
					break;
				}
			}

			let mut rangeproof_identifier_iter = SegmentIdentifier::traversal_iter(
				self.archive_header.output_mmr_size,
				self.default_rangeproof_segment_height,
			);

			while let Some(rp_id) = rangeproof_identifier_iter.next() {
				// Advance output iterator to next needed position
				if rp_id
					.segment_pos_range(self.archive_header.output_mmr_size)
					.1 <= local_rangeproof_mmr_size
				{
					continue;
				}
				// Break if we're full
				if return_vec.len() > max_elements {
					break;
				}

				if !self.has_rangeproof_segment_with_id(rp_id) {
					return_vec.push(SegmentTypeIdentifier::new(SegmentType::RangeProof, rp_id));
					// Let other trees have a chance to put in a segment request
					break;
				}
			}
		}
		if return_vec.is_empty() && self.bitmap_cache.is_some() {
			self.all_segments_complete = true;
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
			"pibd_desegmenter: finalizing and caching bitmap - accumulator root: {}",
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
			"pibd_desegmenter - expected number of leaves in bitmap MMR: {}",
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
			"pibd_desegmenter - expected size of bitmap MMR: {}",
			self.bitmap_mmr_size
		);
	}

	/// Cache a bitmap segment if we don't already have it
	fn cache_bitmap_segment(&mut self, in_seg: Segment<BitmapChunk>) {
		if self
			.bitmap_segment_cache
			.iter()
			.find(|i| i.identifier() == in_seg.identifier())
			.is_none()
		{
			self.bitmap_segment_cache.push(in_seg);
		}
	}

	/// Whether our list already contains this bitmap segment
	fn has_bitmap_segment_with_id(&self, seg_id: SegmentIdentifier) -> bool {
		self.bitmap_segment_cache
			.iter()
			.find(|i| i.identifier() == seg_id)
			.is_some()
	}

	/// Return an identifier for the next segment we need for the bitmap pmmr
	fn next_required_bitmap_segment_index(&self) -> Option<u64> {
		let local_bitmap_pmmr_size = self.bitmap_accumulator.readonly_pmmr().unpruned_size();
		let cur_segment_count = SegmentIdentifier::count_segments_required(
			local_bitmap_pmmr_size,
			self.default_bitmap_segment_height,
		);
		let total_segment_count = SegmentIdentifier::count_segments_required(
			self.bitmap_mmr_size,
			self.default_bitmap_segment_height,
		);
		if cur_segment_count == total_segment_count {
			None
		} else {
			Some(cur_segment_count as u64)
		}
	}

	/// Adds and validates a bitmap chunk
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
		debug!("pibd_desegmenter: adding segment to cache");
		// All okay, add to our cached list of bitmap segments
		self.cache_bitmap_segment(segment);
		Ok(())
	}

	/// Apply a bitmap segment at the index cache
	pub fn apply_bitmap_segment(&mut self, idx: usize) -> Result<(), Error> {
		let segment = self.bitmap_segment_cache.remove(idx);
		debug!(
			"pibd_desegmenter: apply bitmap segment at segment idx {}",
			segment.identifier().idx
		);
		// Add leaves to bitmap accumulator
		let (_sid, _hash_pos, _hashes, _leaf_pos, leaf_data, _proof) = segment.parts();
		for chunk in leaf_data.into_iter() {
			self.bitmap_accumulator.append_chunk(chunk)?;
		}
		Ok(())
	}

	/// Whether our list already contains this bitmap segment
	fn has_output_segment_with_id(&self, seg_id: SegmentIdentifier) -> bool {
		self.output_segment_cache
			.iter()
			.find(|i| i.identifier() == seg_id)
			.is_some()
	}

	/// Cache an output segment if we don't already have it
	fn cache_output_segment(&mut self, in_seg: Segment<OutputIdentifier>) {
		if self
			.output_segment_cache
			.iter()
			.find(|i| i.identifier() == in_seg.identifier())
			.is_none()
		{
			self.output_segment_cache.push(in_seg);
		}
	}

	/// Apply an output segment at the index cache
	pub fn apply_output_segment(&mut self, idx: usize) -> Result<(), Error> {
		let segment = self.output_segment_cache.remove(idx);
		debug!(
			"pibd_desegmenter: applying output segment at segment idx {}",
			segment.identifier().idx
		);
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

	/// Return an identifier for the next segment we need for the output pmmr
	fn next_required_output_segment_index(&self) -> Option<u64> {
		let local_output_mmr_size;
		{
			let txhashset = self.txhashset.read();
			local_output_mmr_size = txhashset.output_mmr_size();
		}

		// Special case here. If the mmr size is 1, this is a fresh chain
		// with naught but a humble genesis block. We need segment 0, (and
		// also need to skip the genesis block when applying the segment)

		let cur_segment_count = if local_output_mmr_size == 1 {
			0
		} else {
			SegmentIdentifier::count_segments_required(
				local_output_mmr_size,
				self.default_output_segment_height,
			)
		};

		let total_segment_count = SegmentIdentifier::count_segments_required(
			self.archive_header.output_mmr_size,
			self.default_output_segment_height,
		);
		trace!(
			"Next required output segment is {} of {}",
			cur_segment_count,
			total_segment_count
		);
		if cur_segment_count == total_segment_count {
			None
		} else {
			Some(cur_segment_count as u64)
		}
	}

	/// Adds a output segment
	pub fn add_output_segment(
		&mut self,
		segment: Segment<OutputIdentifier>,
		bitmap_root: Option<Hash>,
	) -> Result<(), Error> {
		debug!("pibd_desegmenter: add output segment");
		// TODO: This, something very wrong, probably need to reset entire body sync
		// check bitmap root matches what we already have
		/*if bitmap_root != Some(self.bitmap_accumulator.root()) {

		}*/
		segment.validate_with(
			self.archive_header.output_mmr_size, // Last MMR pos at the height being validated
			self.bitmap_cache.as_ref(),
			self.archive_header.output_root, // Output root we're checking for
			self.archive_header.output_mmr_size,
			self.bitmap_accumulator.root(), // Other root
			false,
		)?;
		self.cache_output_segment(segment);
		Ok(())
	}

	/// Whether our list already contains this rangeproof segment
	fn has_rangeproof_segment_with_id(&self, seg_id: SegmentIdentifier) -> bool {
		self.rangeproof_segment_cache
			.iter()
			.find(|i| i.identifier() == seg_id)
			.is_some()
	}

	/// Cache a RangeProof segment if we don't already have it
	fn cache_rangeproof_segment(&mut self, in_seg: Segment<RangeProof>) {
		if self
			.rangeproof_segment_cache
			.iter()
			.find(|i| i.identifier() == in_seg.identifier())
			.is_none()
		{
			self.rangeproof_segment_cache.push(in_seg);
		}
	}

	/// Apply a rangeproof segment at the index cache
	pub fn apply_rangeproof_segment(&mut self, idx: usize) -> Result<(), Error> {
		let segment = self.rangeproof_segment_cache.remove(idx);
		debug!(
			"pibd_desegmenter: applying rangeproof segment at segment idx {}",
			segment.identifier().idx
		);
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

	/// Return an identifier for the next segment we need for the rangeproof pmmr
	fn next_required_rangeproof_segment_index(&self) -> Option<u64> {
		let local_rangeproof_mmr_size;
		{
			let txhashset = self.txhashset.read();
			local_rangeproof_mmr_size = txhashset.rangeproof_mmr_size();
		}

		// Special case here. If the mmr size is 1, this is a fresh chain
		// with naught but a humble genesis block. We need segment 0, (and
		// also need to skip the genesis block when applying the segment)

		let cur_segment_count = if local_rangeproof_mmr_size == 1 {
			0
		} else {
			SegmentIdentifier::count_segments_required(
				local_rangeproof_mmr_size,
				self.default_rangeproof_segment_height,
			)
		};

		let total_segment_count = SegmentIdentifier::count_segments_required(
			self.archive_header.output_mmr_size,
			self.default_rangeproof_segment_height,
		);
		trace!(
			"Next required rangeproof segment is {} of {}",
			cur_segment_count,
			total_segment_count
		);
		if cur_segment_count == total_segment_count {
			None
		} else {
			Some(cur_segment_count as u64)
		}
	}

	/// Adds a Rangeproof segment
	pub fn add_rangeproof_segment(&mut self, segment: Segment<RangeProof>) -> Result<(), Error> {
		debug!("pibd_desegmenter: add rangeproof segment");
		segment.validate(
			self.archive_header.output_mmr_size, // Last MMR pos at the height being validated
			self.bitmap_cache.as_ref(),
			self.archive_header.range_proof_root, // Range proof root we're checking for
		)?;
		self.cache_rangeproof_segment(segment);
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
