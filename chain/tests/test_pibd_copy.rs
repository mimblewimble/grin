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

use grin_chain as chain;
use grin_core as core;
use grin_util as util;

#[macro_use]
extern crate log;

use std::sync::Arc;

use crate::chain::txhashset::{BitmapAccumulator, BitmapChunk, BitmapSegment};
use crate::chain::types::{NoopAdapter, Options};
use crate::core::core::{hash::Hashed, pmmr::segment::SegmentIdentifier};
use crate::core::core::{pmmr, Segment};
use crate::core::{global, pow};

use croaring::Bitmap;

mod chain_test_helper;

use self::chain_test_helper::clean_output_dir;

#[test]
fn test_pibd_copy() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	let genesis = pow::mine_genesis_block().unwrap();
	// Note there is now a 'test' in grin_wallet that can be manually tweaked to create a
	// small testing chain with actual transaction data
	let src_root_dir = format!("./chain/tests/test_data/chain_compacted");
	let dest_root_dir = format!("./chain/tests/test_output/.segment_copy");
	clean_output_dir(&dest_root_dir);
	{
		debug!("Reading Chain, genesis block: {}", genesis.hash());
		let dummy_adapter = Arc::new(NoopAdapter {});

		// The original chain we're reading from
		let src_chain = Arc::new(
			chain::Chain::init(
				src_root_dir.clone(),
				dummy_adapter.clone(),
				genesis.clone(),
				pow::verify_size,
				false,
			)
			.unwrap(),
		);

		// And the output chain we're writing to
		let dest_chain = Arc::new(
			chain::Chain::init(
				dest_root_dir.clone(),
				dummy_adapter,
				genesis.clone(),
				pow::verify_size,
				false,
			)
			.unwrap(),
		);

		//TODO: Later test
		//src_chain.compact().unwrap();
		/*src_chain
		.validate(true)
		.expect("Source chain validation failed, stop");*/

		let options = Options::NONE;
		let sh = src_chain.get_header_by_height(0).unwrap();
		debug!("S Genesis - {}", sh.hash());
		let dh = dest_chain.get_header_by_height(0).unwrap();
		debug!("D Genesis - {}", dh.hash());

		let horizon_height = 110;

		// Copy the header from source to output
		for h in 1..=horizon_height {
			let h = src_chain.get_header_by_height(h).unwrap();
			dest_chain.process_block_header(&h, options).unwrap();
		}

		let src_header_head = src_chain.header_head().unwrap();
		let dest_header_head = dest_chain.header_head().unwrap();

		debug!(
			"Source Header Tip - Height: {} Prev Hash: {}",
			src_header_head.height, src_header_head.last_block_h
		);
		debug!(
			"Dest Header Tip - Height: {} Prev Hash: {}",
			dest_header_head.height, dest_header_head.prev_block_h
		);

		// Archive header for this test data is at 110
		let dest_horizon_header = src_chain.get_header_by_height(horizon_height).unwrap();
		debug!("Horizon Header: {}", dest_horizon_header.hash());

		debug!(
			"Dest horizon header {} output root: {}",
			horizon_height, dest_horizon_header.output_root
		);
		debug!(
			"Dest horizon  header {} range proof root: {}",
			horizon_height, dest_horizon_header.range_proof_root
		);
		debug!(
			"Dest horizon header {} kernel root: {}",
			horizon_height, dest_horizon_header.kernel_root
		);
		debug!(
			"Dest horizon header {} kernel mmr size: {}",
			horizon_height, dest_horizon_header.kernel_mmr_size
		);
		debug!(
			"Dest horizon header {} output mmr size: {}",
			horizon_height, dest_horizon_header.output_mmr_size
		);

		// Init segmenter, (note this still has to be lazy init somewhere on a peer)
		let segmenter = src_chain.segmenter().unwrap();

		// Last MMR position according to the target header
		let last_pos = dest_horizon_header.kernel_mmr_size;

		// Height at which to read kernel segments (lower than thresholds defined in spec - for testing)
		let target_segment_height = 3;

		// KERNELS - Read + Validate
		// Build up a list of identifiers we'll need in order to reconstruct the MMR
		// defined at the horizon head
		// TODO: This can probably be derived from the PMMR we'll eventually be building
		// (check if total size is equal to total size at horizon header)
		let identifier_iter = SegmentIdentifier::traversal_iter(
			dest_horizon_header.kernel_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			debug!("Getting kernel segment with Segment Identifier {:?}", sid);
			let kernel_segment = segmenter.kernel_segment(sid).unwrap();
			// Validate Kernel segment (which does not require a bitmap)
			if let Err(e) = kernel_segment.validate(last_pos, None, dest_horizon_header.kernel_root)
			{
				panic!("Unable to validate kernel_segment root: {}", e);
			}
		}

		// BITMAP - Read + Validate
		// TODO: Check this calc
		let bitmap_mmr_num_leaves = pmmr::n_leaves(dest_horizon_header.output_mmr_size / 1024) + 1;
		let bitmap_pmmr_size = pmmr::insertion_to_pmmr_index(bitmap_mmr_num_leaves);
		let identifier_iter =
			SegmentIdentifier::traversal_iter(bitmap_mmr_num_leaves, target_segment_height);

		for sid in identifier_iter {
			debug!("Getting bitmap segment with Segment Identifier {:?}", sid);
			let (bitmap_segment, output_root_hash) = segmenter.bitmap_segment(sid).unwrap();
			debug!(
				"Bitmap segmenter reports output root hash is {:?}",
				output_root_hash
			);
			// Validate bitmap segment with provided output hash
			if let Err(e) = bitmap_segment.validate_with(
				bitmap_pmmr_size, // Last MMR pos at the height being validated, in this case of the bitmap root
				None,
				dest_horizon_header.output_root, // Output root we're checking for
				dest_horizon_header.output_mmr_size,
				output_root_hash, // Other root
				true,
			) {
				panic!("Unable to validate bitmap_root: {}", e);
			}
		}

		// OUTPUTS  - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			dest_horizon_header.output_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			debug!("Getting output segment with Segment Identifier {:?}", sid);
			let (output_segment, bitmap_root_hash) = segmenter.output_segment(sid).unwrap();
			debug!(
				"Output segmenter reports bitmap hash is {:?}",
				bitmap_root_hash
			);
			// Validate Output
			if let Err(e) = output_segment.validate_with(
				dest_horizon_header.output_mmr_size, // Last MMR pos at the height being validated
				// TODO: Need to provide Bitmap???
				None,
				dest_horizon_header.output_root, // Output root we're checking for
				dest_horizon_header.output_mmr_size,
				bitmap_root_hash, // Other root
				false,
			) {
				panic!("Unable to validate output segment root: {}", e);
			}
		}

		// PROOFS  - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			dest_horizon_header.output_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			debug!(
				"Getting rangeproof segment with Segment Identifier {:?}",
				sid
			);
			let rangeproof_segment = segmenter.rangeproof_segment(sid).unwrap();
			// Validate Kernel segment (which does not require a bitmap)
			if let Err(e) = rangeproof_segment.validate(
				dest_horizon_header.output_mmr_size, // Last MMR pos at the height being validated
				// TODO: Need to provide Bitmap???
				None,
				dest_horizon_header.range_proof_root, // Output root we're checking for
			) {
				panic!("Unable to validate rangeproof segment root: {}", e);
			}
		}

		/*
		// Recreate a bitmap from the given chunks
		let bitmap = output_bitmap_segment.as_bitmap();
		debug!("BIT MAP CONTAINS: {}", bitmap.contains(127));
		*/

		println!("urf");
	}
}
