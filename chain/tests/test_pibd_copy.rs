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
	let src_root_dir = format!("./chain/tests/test_data/chain_raw");
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

		let options = Options::NONE;
		let sh = src_chain.get_header_by_height(0).unwrap();
		debug!("S Genesis - {}", sh.hash());

		let dh = dest_chain.get_header_by_height(0).unwrap();
		debug!("D Genesis - {}", dh.hash());

		let first_segment_height = 2;

		// Copy the header from source to output
		for h in 1..=first_segment_height + 1 {
			let h = src_chain.get_header_by_height(h).unwrap();
			debug!("Src Block 1 prev hash -> {}", h.prev_hash);
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

		// Need last completed header for roots
		let dest_root_header = dest_chain
			.get_block_header(&dest_header_head.prev_block_h)
			.unwrap();
		debug!("Dest header output root: {}", dest_root_header.output_root);
		debug!(
			"Dest header range proof root: {}",
			dest_root_header.range_proof_root
		);
		debug!("Dest header kernel root: {}", dest_root_header.kernel_root);

		let first_segment_header = dest_chain
			.get_header_by_height(first_segment_height)
			.unwrap();
		debug!(
			"Dest header {} output root: {}",
			first_segment_height, first_segment_header.output_root
		);
		debug!(
			"Dest header {} range proof root: {}",
			first_segment_height, first_segment_header.range_proof_root
		);
		debug!(
			"Dest header {} kernel root: {}",
			first_segment_height, first_segment_header.kernel_root
		);
		debug!(
			"Dest header {} kernel mmr size: {}",
			first_segment_height, first_segment_header.kernel_mmr_size
		);
		debug!(
			"Dest header {} output mmr size: {}",
			first_segment_height, first_segment_header.output_mmr_size
		);

		// Archive header is at 110 for this particular data set

		// Get all PMMR segments from the source up to segment height
		let sid = SegmentIdentifier {
			height: first_segment_height as u8,
			idx: 0,
		};
		let segmenter = src_chain.segmenter().unwrap();
		let output_segment = segmenter.output_segment(sid).unwrap();
		let kernel_segment = segmenter.kernel_segment(sid).unwrap();
		let bitmap_segment = segmenter.bitmap_segment(sid).unwrap();
		let rangeproof_segment = segmenter.rangeproof_segment(sid).unwrap();

		// Last MMR position according to the target header
		let last_pos = first_segment_header.kernel_mmr_size;

		// Validate Kernel segment (which does not require a bitmap)
		let (
			kernel_sid,
			_kernel_hash_pos,
			_kernel_hashes,
			kernel_leaf_pos,
			kernel_leaf_data,
			kernel_proof,
		) = kernel_segment.clone().parts();

		let kernel_segment_root_1 = kernel_segment.root(last_pos, None).unwrap().unwrap();
		debug!("Kernel segment root 1: {}", kernel_segment_root_1);
		if let Err(e) = kernel_segment.validate(last_pos, None, first_segment_header.kernel_root) {
			panic!("Unable to validate kernel_segment_root");
		}

		// Retrieve the output bitmap segment, as well as convert to a bitmap
		// for use in further output/rangeproof validation. Note this can't be
		// validated on its own as its root is hashed with output_root in the block header
		let (
			bitmap_sid,
			_bitmap_hash_pos,
			_bitmap_hashes,
			bitmap_leaf_pos,
			bitmap_leaf_data,
			bitmap_proof,
		) = bitmap_segment.0.clone().parts();

		debug!("BITMAP_LEAF_POS: {:?}", bitmap_leaf_pos);
		debug!("BITMAP_LEAF_DATA: {:?}", bitmap_leaf_data);

		let bitmap_segment_root_1 = bitmap_segment
			.0
			.root(*bitmap_leaf_pos.last().unwrap(), None)
			.unwrap()
			.unwrap();
		debug!("Bitmap segment root 1: {}", bitmap_segment_root_1);

		let output_bitmap_segment: BitmapSegment = bitmap_segment.0.into();

		let output_bitmap_output_root = bitmap_segment.1;
		debug!(
			"OUTPUT ROOT FROM BITMAP SEGMENT: {}",
			output_bitmap_output_root
		);

		// Recreate a bitmap from the given chunks
		let bitmap = output_bitmap_segment.as_bitmap();
		debug!("BIT MAP CONTAINS: {}", bitmap.contains(127));

		// Validate Rangeproof segment (which requires a bitmap when pruned) but
		// can be valiated directly
		let rangeproof_segment_root_1 = rangeproof_segment
			.root(last_pos, Some(&bitmap))
			.unwrap()
			.unwrap();
		debug!("Rangeproof segment root 1: {}", rangeproof_segment_root_1);
		if let Err(e) =
			rangeproof_segment.validate(last_pos, None, first_segment_header.range_proof_root)
		{
			panic!("Unable to validate rangeproof_segment_root");
		}

		// Test output segment
		let (
			output_sid,
			output_hash_pos,
			_output_hashes,
			output_leaf_pos,
			output_leaf_data,
			output_proof,
		) = output_segment.0.clone().parts();

		// Test get output segment root
		let output_segment_root_1 = output_segment
			.0
			.root(last_pos, Some(&bitmap))
			.unwrap()
			.unwrap();

		debug!("Output segment root 1: {}", output_segment_root_1);
		debug!("Output hash pos: {:?}", output_hash_pos);

		let output_bitmap_root = output_segment.1;
		debug!("BITMAP ROOT FROM OUTPUT SEGMENT: {}", output_bitmap_root);

		// Now validate both together
		if let Err(e) = output_segment.0.validate_with(
			last_pos,                             // Last MMR pos at the height being validated
			Some(&bitmap),                        // Bitmap recreated from segments
			first_segment_header.output_root,     // Output root we're checking for
			first_segment_header.output_mmr_size, //
			output_bitmap_root,
			false,
		) {
			panic!("Unable to validate output_root");
		}

		println!("urf");
	}
}
