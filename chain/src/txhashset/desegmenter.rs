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

use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::{pmmr, pmmr::ReadablePMMR};
use crate::core::core::{
	BlockHeader, BlockSums, OutputIdentifier, Segment, SegmentIdentifier, SegmentType,
	SegmentTypeIdentifier, TxKernel,
};
use crate::error::Error;
use crate::txhashset::{BitmapAccumulator, BitmapChunk, TxHashSet};
use crate::types::{Tip, TxHashsetWriteStatus};
use crate::util::secp::pedersen::RangeProof;
use crate::util::{RwLock, StopState};
use crate::SyncState;

use crate::pibd_params;
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

	genesis: BlockHeader,

	default_bitmap_segment_height: u8,
	default_output_segment_height: u8,
	default_rangeproof_segment_height: u8,
	default_kernel_segment_height: u8,

	bitmap_accumulator: BitmapAccumulator,
	bitmap_segment_cache: Vec<Segment<BitmapChunk>>,
	output_segment_cache: Vec<Segment<OutputIdentifier>>,
	rangeproof_segment_cache: Vec<Segment<RangeProof>>,
	kernel_segment_cache: Vec<Segment<TxKernel>>,

	bitmap_mmr_leaf_count: u64,
	bitmap_mmr_size: u64,

	/// Maximum number of segments to cache before we stop requesting others
	max_cached_segments: usize,

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
		genesis: BlockHeader,
		store: Arc<store::ChainStore>,
	) -> Desegmenter {
		trace!("Creating new desegmenter");
		let mut retval = Desegmenter {
			txhashset,
			header_pmmr,
			archive_header,
			store,
			genesis,
			bitmap_accumulator: BitmapAccumulator::new(),
			default_bitmap_segment_height: pibd_params::BITMAP_SEGMENT_HEIGHT,
			default_output_segment_height: pibd_params::OUTPUT_SEGMENT_HEIGHT,
			default_rangeproof_segment_height: pibd_params::RANGEPROOF_SEGMENT_HEIGHT,
			default_kernel_segment_height: pibd_params::KERNEL_SEGMENT_HEIGHT,
			bitmap_segment_cache: vec![],
			output_segment_cache: vec![],
			rangeproof_segment_cache: vec![],
			kernel_segment_cache: vec![],

			bitmap_mmr_leaf_count: 0,
			bitmap_mmr_size: 0,

			max_cached_segments: pibd_params::MAX_CACHED_SEGMENTS,

			bitmap_cache: None,

			all_segments_complete: false,
		};
		retval.calc_bitmap_mmr_sizes();
		retval
	}

	/// Reset all state
	pub fn reset(&mut self) {
		self.all_segments_complete = false;
		self.bitmap_segment_cache = vec![];
		self.output_segment_cache = vec![];
		self.rangeproof_segment_cache = vec![];
		self.kernel_segment_cache = vec![];
		self.bitmap_mmr_leaf_count = 0;
		self.bitmap_mmr_size = 0;
		self.bitmap_cache = None;
		self.bitmap_accumulator = BitmapAccumulator::new();
		self.calc_bitmap_mmr_sizes();
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

	/// Check progress, update status if needed, returns true if all required
	/// segments are in place
	pub fn check_progress(&self, status: Arc<SyncState>) -> Result<bool, Error> {
		let mut latest_block_height = 0;

		let local_output_mmr_size;
		let local_kernel_mmr_size;
		let local_rangeproof_mmr_size;
		{
			let txhashset = self.txhashset.read();
			local_output_mmr_size = txhashset.output_mmr_size();
			local_kernel_mmr_size = txhashset.kernel_mmr_size();
			local_rangeproof_mmr_size = txhashset.rangeproof_mmr_size();
		}

		// going to try presenting PIBD progress as total leaves downloaded
		// total segments probably doesn't make much sense since the segment
		// sizes will be able to change over time, and representative block height
		// can be too lopsided if one pmmr completes faster, so perhaps just
		// use total leaves downloaded and display as a percentage
		let completed_leaves = pmmr::n_leaves(local_output_mmr_size)
			+ pmmr::n_leaves(local_rangeproof_mmr_size)
			+ pmmr::n_leaves(local_kernel_mmr_size);

		// Find latest 'complete' header.
		// First take lesser of rangeproof and output mmr sizes
		let latest_output_size = std::cmp::min(local_output_mmr_size, local_rangeproof_mmr_size);

		// Find first header in which 'output_mmr_size' and 'kernel_mmr_size' are greater than
		// given sizes

		let res = {
			let header_pmmr = self.header_pmmr.read();
			header_pmmr.get_first_header_with(
				latest_output_size,
				local_kernel_mmr_size,
				latest_block_height,
				self.store.clone(),
			)
		};

		if let Some(h) = res {
			latest_block_height = h.height;

			// TODO: Unwraps
			let tip = Tip::from_header(&h);
			let batch = self.store.batch()?;
			batch.save_pibd_head(&tip)?;
			batch.commit()?;

			status.update_pibd_progress(
				false,
				false,
				completed_leaves,
				latest_block_height,
				&self.archive_header,
			);
			if local_kernel_mmr_size == self.archive_header.kernel_mmr_size
				&& local_output_mmr_size == self.archive_header.output_mmr_size
				&& local_rangeproof_mmr_size == self.archive_header.output_mmr_size
				&& self.bitmap_cache.is_some()
			{
				// All is complete
				return Ok(true);
			}
		}

		Ok(false)
	}

	/// Once the PIBD set is downloaded, we need to ensure that the respective leaf sets
	/// match the bitmap (particularly in the case of outputs being spent after a PIBD catch-up)
	pub fn check_update_leaf_set_state(&self) -> Result<(), Error> {
		let mut header_pmmr = self.header_pmmr.write();
		let mut txhashset = self.txhashset.write();
		let mut _batch = self.store.batch()?;
		txhashset::extending(&mut header_pmmr, &mut txhashset, &mut _batch, |ext, _| {
			let extension = &mut ext.extension;
			if let Some(b) = &self.bitmap_cache {
				extension.update_leaf_sets(&b)?;
			}
			Ok(())
		})?;
		Ok(())
	}

	/// This is largely copied from chain.rs txhashset_write and related functions,
	/// the idea being that the txhashset version will eventually be removed
	pub fn validate_complete_state(
		&self,
		status: Arc<SyncState>,
		stop_state: Arc<StopState>,
	) -> Result<(), Error> {
		// Quick root check first:
		{
			let txhashset = self.txhashset.read();
			txhashset.roots().validate(&self.archive_header)?;
		}

		// TODO: Possibly Keep track of this in the DB so we can pick up where we left off if needed
		let last_rangeproof_validation_pos = 0;

		// Validate kernel history
		{
			debug!("desegmenter validation: rewinding and validating kernel history (readonly)");
			let txhashset = self.txhashset.read();
			let mut count = 0;
			let mut current = self.archive_header.clone();
			let total = current.height;
			txhashset::rewindable_kernel_view(&txhashset, |view, batch| {
				while current.height > 0 {
					view.rewind(&current)?;
					view.validate_root()?;
					current = batch.get_previous_header(&current)?;
					count += 1;
					if current.height % 100000 == 0 || current.height == total {
						status.on_setup(Some(total - current.height), Some(total), None, None);
					}
					if stop_state.is_stopped() {
						return Ok(());
					}
				}
				Ok(())
			})?;
			debug!(
				"desegmenter validation: validated kernel root on {} headers",
				count,
			);
		}

		if stop_state.is_stopped() {
			return Ok(());
		}

		// Check kernel MMR root for every block header.
		// Check NRD relative height rules for full kernel history.

		{
			let header_pmmr = self.header_pmmr.read();
			let txhashset = self.txhashset.read();
			let batch = self.store.batch()?;
			txhashset.verify_kernel_pos_index(
				&self.genesis,
				&header_pmmr,
				&batch,
				Some(status.clone()),
				Some(stop_state.clone()),
			)?;
		}

		if stop_state.is_stopped() {
			return Ok(());
		}

		status.on_setup(None, None, None, None);
		// Prepare a new batch and update all the required records
		{
			debug!("desegmenter validation: rewinding a 2nd time (writeable)");
			let mut header_pmmr = self.header_pmmr.write();
			let mut txhashset = self.txhashset.write();
			let mut batch = self.store.batch()?;
			txhashset::extending(
				&mut header_pmmr,
				&mut txhashset,
				&mut batch,
				|ext, batch| {
					let extension = &mut ext.extension;
					extension.rewind(&self.archive_header, batch)?;

					// Validate the extension, generating the utxo_sum and kernel_sum.
					// Full validation, including rangeproofs and kernel signature verification.
					let (utxo_sum, kernel_sum) = extension.validate(
						&self.genesis,
						false,
						&*status,
						Some(last_rangeproof_validation_pos),
						None,
						&self.archive_header,
						Some(stop_state.clone()),
					)?;

					if stop_state.is_stopped() {
						return Ok(());
					}

					// Save the block_sums (utxo_sum, kernel_sum) to the db for use later.
					batch.save_block_sums(
						&self.archive_header.hash(),
						BlockSums {
							utxo_sum,
							kernel_sum,
						},
					)?;

					Ok(())
				},
			)?;

			if stop_state.is_stopped() {
				return Ok(());
			}

			debug!("desegmenter_validation: finished validating and rebuilding");
			status.on_save();

			{
				// Save the new head to the db and rebuild the header by height index.
				let tip = Tip::from_header(&self.archive_header);

				batch.save_body_head(&tip)?;

				// Reset the body tail to the body head after a txhashset write
				batch.save_body_tail(&tip)?;
			}

			// Rebuild our output_pos index in the db based on fresh UTXO set.
			txhashset.init_output_pos_index(&header_pmmr, &batch)?;

			// Rebuild our NRD kernel_pos index based on recent kernel history.
			txhashset.init_recent_kernel_pos_index(&header_pmmr, &batch)?;

			// Commit all the changes to the db.
			batch.commit()?;

			debug!("desegmenter_validation: finished committing the batch (head etc.)");

			status.on_done();
		}
		Ok(())
	}

	/// Apply next set of segments that are ready to be appended to their respective trees,
	/// and kick off any validations that can happen.
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

			// Check if we can apply the next output segment(s)
			if let Some(next_output_idx) = self.next_required_output_segment_index() {
				if let Some((idx, _seg)) = self
					.output_segment_cache
					.iter()
					.enumerate()
					.find(|s| s.1.identifier().idx == next_output_idx)
				{
					self.apply_output_segment(idx)?;
				}
			} else {
				if self.output_segment_cache.len() >= self.max_cached_segments {
					self.output_segment_cache = vec![];
				}
			}
			// Check if we can apply the next rangeproof segment
			if let Some(next_rp_idx) = self.next_required_rangeproof_segment_index() {
				if let Some((idx, _seg)) = self
					.rangeproof_segment_cache
					.iter()
					.enumerate()
					.find(|s| s.1.identifier().idx == next_rp_idx)
				{
					self.apply_rangeproof_segment(idx)?;
				}
			} else {
				if self.rangeproof_segment_cache.len() >= self.max_cached_segments {
					self.rangeproof_segment_cache = vec![];
				}
			}
			// Check if we can apply the next kernel segment
			if let Some(next_kernel_idx) = self.next_required_kernel_segment_index() {
				if let Some((idx, _seg)) = self
					.kernel_segment_cache
					.iter()
					.enumerate()
					.find(|s| s.1.identifier().idx == next_kernel_idx)
				{
					self.apply_kernel_segment(idx)?;
				}
			} else {
				if self.kernel_segment_cache.len() >= self.max_cached_segments {
					self.kernel_segment_cache = vec![];
				}
			}
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
			// bitmap, now continue with other segments, evenly spreading requests
			// among MMRs
			let local_output_mmr_size;
			let local_kernel_mmr_size;
			let local_rangeproof_mmr_size;
			{
				let txhashset = self.txhashset.read();
				local_output_mmr_size = txhashset.output_mmr_size();
				local_kernel_mmr_size = txhashset.kernel_mmr_size();
				local_rangeproof_mmr_size = txhashset.rangeproof_mmr_size();
			}
			// TODO: Fix, alternative approach, this is very inefficient
			let mut output_identifier_iter = SegmentIdentifier::traversal_iter(
				self.archive_header.output_mmr_size,
				self.default_output_segment_height,
			);

			let mut elems_added = 0;
			while let Some(output_id) = output_identifier_iter.next() {
				// Advance output iterator to next needed position
				let (_first, last) =
					output_id.segment_pos_range(self.archive_header.output_mmr_size);
				if last <= local_output_mmr_size {
					continue;
				}
				if self.output_segment_cache.len() >= self.max_cached_segments {
					break;
				}
				if !self.has_output_segment_with_id(output_id) {
					return_vec.push(SegmentTypeIdentifier::new(SegmentType::Output, output_id));
					elems_added += 1;
				}
				if elems_added == max_elements / 3 {
					break;
				}
			}

			let mut rangeproof_identifier_iter = SegmentIdentifier::traversal_iter(
				self.archive_header.output_mmr_size,
				self.default_rangeproof_segment_height,
			);

			elems_added = 0;
			while let Some(rp_id) = rangeproof_identifier_iter.next() {
				let (_first, last) = rp_id.segment_pos_range(self.archive_header.output_mmr_size);
				// Advance rangeproof iterator to next needed position
				if last <= local_rangeproof_mmr_size {
					continue;
				}
				if self.rangeproof_segment_cache.len() >= self.max_cached_segments {
					break;
				}
				if !self.has_rangeproof_segment_with_id(rp_id) {
					return_vec.push(SegmentTypeIdentifier::new(SegmentType::RangeProof, rp_id));
					elems_added += 1;
				}
				if elems_added == max_elements / 3 {
					break;
				}
			}

			let mut kernel_identifier_iter = SegmentIdentifier::traversal_iter(
				self.archive_header.kernel_mmr_size,
				self.default_kernel_segment_height,
			);

			elems_added = 0;
			while let Some(k_id) = kernel_identifier_iter.next() {
				// Advance kernel iterator to next needed position
				let (_first, last) = k_id.segment_pos_range(self.archive_header.kernel_mmr_size);
				// Advance rangeproof iterator to next needed position
				if last <= local_kernel_mmr_size {
					continue;
				}
				if self.kernel_segment_cache.len() >= self.max_cached_segments {
					break;
				}
				if !self.has_kernel_segment_with_id(k_id) {
					return_vec.push(SegmentTypeIdentifier::new(SegmentType::Kernel, k_id));
					elems_added += 1;
				}
				if elems_added == max_elements / 3 {
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
	pub fn finalize_bitmap(&mut self) -> Result<(), Error> {
		trace!(
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
		trace!(
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

		trace!(
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
		trace!("pibd_desegmenter: add bitmap segment");
		segment.validate_with(
			self.bitmap_mmr_size, // Last MMR pos at the height being validated, in this case of the bitmap root
			None,
			self.archive_header.output_root, // Output root we're checking for
			self.archive_header.output_mmr_size,
			output_root_hash, // Other root
			true,
		)?;
		trace!("pibd_desegmenter: adding segment to cache");
		// All okay, add to our cached list of bitmap segments
		self.cache_bitmap_segment(segment);
		Ok(())
	}

	/// Apply a bitmap segment at the index cache
	pub fn apply_bitmap_segment(&mut self, idx: usize) -> Result<(), Error> {
		let segment = self.bitmap_segment_cache.remove(idx);
		trace!(
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
		trace!(
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
		// note this is implementation-specific, the code for creating
		// a new chain creates the genesis block pmmr entries by default

		let mut cur_segment_count = if local_output_mmr_size == 1 {
			0
		} else {
			SegmentIdentifier::count_segments_required(
				local_output_mmr_size,
				self.default_output_segment_height,
			)
		};

		// When resuming, we need to ensure we're getting the previous segment if needed
		let theoretical_pmmr_size =
			SegmentIdentifier::pmmr_size(cur_segment_count, self.default_output_segment_height);
		if local_output_mmr_size < theoretical_pmmr_size {
			cur_segment_count -= 1;
		}

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
		_bitmap_root: Option<Hash>,
	) -> Result<(), Error> {
		trace!("pibd_desegmenter: add output segment");
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
		trace!(
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

		let mut cur_segment_count = if local_rangeproof_mmr_size == 1 {
			0
		} else {
			SegmentIdentifier::count_segments_required(
				local_rangeproof_mmr_size,
				self.default_rangeproof_segment_height,
			)
		};

		// When resuming, we need to ensure we're getting the previous segment if needed
		let theoretical_pmmr_size =
			SegmentIdentifier::pmmr_size(cur_segment_count, self.default_rangeproof_segment_height);
		if local_rangeproof_mmr_size < theoretical_pmmr_size {
			cur_segment_count -= 1;
		}

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
		trace!("pibd_desegmenter: add rangeproof segment");
		segment.validate(
			self.archive_header.output_mmr_size, // Last MMR pos at the height being validated
			self.bitmap_cache.as_ref(),
			self.archive_header.range_proof_root, // Range proof root we're checking for
		)?;
		self.cache_rangeproof_segment(segment);
		Ok(())
	}

	/// Whether our list already contains this kernel segment
	fn has_kernel_segment_with_id(&self, seg_id: SegmentIdentifier) -> bool {
		self.kernel_segment_cache
			.iter()
			.find(|i| i.identifier() == seg_id)
			.is_some()
	}

	/// Cache a Kernel segment if we don't already have it
	fn cache_kernel_segment(&mut self, in_seg: Segment<TxKernel>) {
		if self
			.kernel_segment_cache
			.iter()
			.find(|i| i.identifier() == in_seg.identifier())
			.is_none()
		{
			self.kernel_segment_cache.push(in_seg);
		}
	}

	/// Apply a kernel segment at the index cache
	pub fn apply_kernel_segment(&mut self, idx: usize) -> Result<(), Error> {
		let segment = self.kernel_segment_cache.remove(idx);
		trace!(
			"pibd_desegmenter: applying kernel segment at segment idx {}",
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
				extension.apply_kernel_segment(segment)?;
				Ok(())
			},
		)?;
		Ok(())
	}

	/// Return an identifier for the next segment we need for the kernel pmmr
	fn next_required_kernel_segment_index(&self) -> Option<u64> {
		let local_kernel_mmr_size;
		{
			let txhashset = self.txhashset.read();
			local_kernel_mmr_size = txhashset.kernel_mmr_size();
		}

		let mut cur_segment_count = if local_kernel_mmr_size == 1 {
			0
		} else {
			SegmentIdentifier::count_segments_required(
				local_kernel_mmr_size,
				self.default_kernel_segment_height,
			)
		};

		// When resuming, we need to ensure we're getting the previous segment if needed
		let theoretical_pmmr_size =
			SegmentIdentifier::pmmr_size(cur_segment_count, self.default_kernel_segment_height);
		if local_kernel_mmr_size < theoretical_pmmr_size {
			cur_segment_count -= 1;
		}

		let total_segment_count = SegmentIdentifier::count_segments_required(
			self.archive_header.kernel_mmr_size,
			self.default_kernel_segment_height,
		);
		trace!(
			"Next required kernel segment is {} of {}",
			cur_segment_count,
			total_segment_count
		);
		if cur_segment_count == total_segment_count {
			None
		} else {
			Some(cur_segment_count as u64)
		}
	}

	/// Adds a Kernel segment
	pub fn add_kernel_segment(&mut self, segment: Segment<TxKernel>) -> Result<(), Error> {
		trace!("pibd_desegmenter: add kernel segment");
		segment.validate(
			self.archive_header.kernel_mmr_size, // Last MMR pos at the height being validated
			None,
			self.archive_header.kernel_root, // Kernel root we're checking for
		)?;
		self.cache_kernel_segment(segment);
		Ok(())
	}
}
