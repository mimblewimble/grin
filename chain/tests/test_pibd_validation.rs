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

use std::sync::Arc;

use crate::chain::txhashset::BitmapAccumulator;
use crate::chain::types::NoopAdapter;
use crate::core::core::pmmr;
use crate::core::core::{hash::Hashed, pmmr::segment::SegmentIdentifier};
use crate::core::{genesis, global, pow};

use croaring::Bitmap;

mod chain_test_helper;

fn test_pibd_chain_validation_impl(is_test_chain: bool, src_root_dir: &str) {
	global::set_local_chain_type(global::ChainTypes::Mainnet);
	let mut genesis = genesis::genesis_main();
	// Height at which to read kernel segments (lower than thresholds defined in spec - for testing)
	let mut target_segment_height = 11;

	if is_test_chain {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
		genesis = pow::mine_genesis_block().unwrap();
		target_segment_height = 3;
	}

	{
		println!("Reading Chain, genesis block: {}", genesis.hash());
		let dummy_adapter = Arc::new(NoopAdapter {});

		// The original chain we're reading from
		let src_chain = Arc::new(
			chain::Chain::init(
				src_root_dir.into(),
				dummy_adapter.clone(),
				genesis.clone(),
				pow::verify_size,
				false,
			)
			.unwrap(),
		);

		// For test compaction purposes
		/*src_chain.compact().unwrap();
		src_chain
		.validate(true)
		.expect("Source chain validation failed, stop");*/

		let sh = src_chain.get_header_by_height(0).unwrap();
		println!("Source Genesis - {}", sh.hash());

		let horizon_header = src_chain.txhashset_archive_header().unwrap();

		println!("Horizon header: {:?}", horizon_header);

		// Copy the header from source to output
		// Not necessary for this test, we're just validating the source
		/*for h in 1..=horizon_height {
			let h = src_chain.get_header_by_height(h).unwrap();
			dest_chain.process_block_header(&h, options).unwrap();
		}*/

		// Init segmenter, (note this still has to be lazy init somewhere on a peer)
		// This is going to use the same block as horizon_header
		let segmenter = src_chain.segmenter().unwrap();

		// BITMAP - Read + Validate, Also recreate bitmap accumulator for target tx hash set
		// Predict number of leaves (chunks) in the bitmap MMR from the number of outputs
		let bitmap_mmr_num_leaves =
			(pmmr::n_leaves(horizon_header.output_mmr_size) as f64 / 1024f64).ceil() as u64;
		println!("BITMAP PMMR NUM_LEAVES: {}", bitmap_mmr_num_leaves);

		// And total size of the bitmap PMMR
		let bitmap_pmmr_size = pmmr::peaks(bitmap_mmr_num_leaves)
			.last()
			.unwrap_or(&pmmr::insertion_to_pmmr_index(bitmap_mmr_num_leaves))
			.clone();
		println!("BITMAP PMMR SIZE: {}", bitmap_pmmr_size);
		println!(
			"Bitmap Segments required: {}",
			SegmentIdentifier::count_segments_required(bitmap_pmmr_size, target_segment_height)
		);
		// TODO: This can probably be derived from the PMMR we'll eventually be building
		// (check if total size is equal to total size at horizon header)
		let identifier_iter =
			SegmentIdentifier::traversal_iter(bitmap_pmmr_size, target_segment_height);

		let mut bitmap_accumulator = BitmapAccumulator::new();
		// Raw bitmap for validation
		let mut bitmap = Bitmap::create();
		let mut chunk_count = 0;

		for sid in identifier_iter {
			println!("Getting bitmap segment with Segment Identifier {:?}", sid);
			let (bitmap_segment, output_root_hash) = segmenter.bitmap_segment(sid).unwrap();
			println!(
				"Bitmap segmenter reports output root hash is {:?}",
				output_root_hash
			);
			// Validate bitmap segment with provided output hash
			if let Err(e) = bitmap_segment.validate_with(
				bitmap_pmmr_size, // Last MMR pos at the height being validated, in this case of the bitmap root
				None,
				horizon_header.output_root, // Output root we're checking for
				horizon_header.output_mmr_size,
				output_root_hash, // Other root
				true,
			) {
				panic!("Unable to validate bitmap_root: {}", e);
			}

			let (_sid, _hash_pos, _hashes, _leaf_pos, leaf_data, _proof) = bitmap_segment.parts();

			// Add to raw bitmap to use in further validation
			for chunk in leaf_data.iter() {
				bitmap.add_many(&chunk.set_iter(chunk_count * 1024).collect::<Vec<u32>>());
				chunk_count += 1;
			}

			// and append to bitmap accumulator
			for chunk in leaf_data.into_iter() {
				bitmap_accumulator.append_chunk(chunk).unwrap();
			}
		}

		println!("Accumulator Root: {}", bitmap_accumulator.root());

		// OUTPUTS  - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			horizon_header.output_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			println!("Getting output segment with Segment Identifier {:?}", sid);
			let (output_segment, bitmap_root_hash) = segmenter.output_segment(sid).unwrap();
			println!(
				"Output segmenter reports bitmap hash is {:?}",
				bitmap_root_hash
			);
			// Validate Output
			if let Err(e) = output_segment.validate_with(
				horizon_header.output_mmr_size, // Last MMR pos at the height being validated
				Some(&bitmap),
				horizon_header.output_root, // Output root we're checking for
				horizon_header.output_mmr_size,
				bitmap_root_hash, // Other root
				false,
			) {
				panic!("Unable to validate output segment root: {}", e);
			}
		}

		// PROOFS  - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			horizon_header.output_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			println!(
				"Getting rangeproof segment with Segment Identifier {:?}",
				sid
			);
			let rangeproof_segment = segmenter.rangeproof_segment(sid).unwrap();
			// Validate Kernel segment (which does not require a bitmap)
			if let Err(e) = rangeproof_segment.validate(
				horizon_header.output_mmr_size, // Last MMR pos at the height being validated
				Some(&bitmap),
				horizon_header.range_proof_root, // Output root we're checking for
			) {
				panic!("Unable to validate rangeproof segment root: {}", e);
			}
		}

		// KERNELS - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			horizon_header.kernel_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			println!("Getting kernel segment with Segment Identifier {:?}", sid);
			let kernel_segment = segmenter.kernel_segment(sid).unwrap();
			// Validate Kernel segment (which does not require a bitmap)
			if let Err(e) = kernel_segment.validate(
				horizon_header.kernel_mmr_size,
				None,
				horizon_header.kernel_root,
			) {
				panic!("Unable to validate kernel_segment root: {}", e);
			}
		}
	}
}

#[test]
// TODO: Fix before merge into master
#[ignore]
fn test_pibd_chain_validation_sample() {
	util::init_test_logger();
	// Note there is now a 'test' in grin_wallet_controller/build_chain
	// that can be manually tweaked to create a
	// small test chain with actual transaction data

	// Test on uncompacted and non-compacted chains
	let src_root_dir = format!("./tests/test_data/chain_raw");
	test_pibd_chain_validation_impl(true, &src_root_dir);
	let src_root_dir = format!("./tests/test_data/chain_compacted");
	test_pibd_chain_validation_impl(true, &src_root_dir);
}

#[test]
#[ignore]
// As above, but run on a real instance of a chain pointed where you like
fn test_pibd_chain_validation_real() {
	util::init_test_logger();
	// if testing against a real chain, insert location here
	let src_root_dir = format!("/Users/yeastplume/Projects/grin_project/server/chain_data");
	test_pibd_chain_validation_impl(false, &src_root_dir);
}
