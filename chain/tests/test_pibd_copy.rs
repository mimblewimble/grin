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

use crate::chain::types::{NoopAdapter, Options};
use crate::core::core::{hash::Hashed, pmmr::segment::SegmentIdentifier};
use crate::core::{genesis, global, pow};

use self::chain_test_helper::clean_output_dir;

mod chain_test_helper;

fn test_pibd_copy_impl(is_test_chain: bool, src_root_dir: &str, dest_root_dir: &str) {
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
		debug!("Reading Chain, genesis block: {}", genesis.hash());
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

		// And the output chain we're writing to
		let dest_chain = Arc::new(
			chain::Chain::init(
				dest_root_dir.into(),
				dummy_adapter,
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
		debug!("Source Genesis - {}", sh.hash());

		let dh = dest_chain.get_header_by_height(0).unwrap();
		debug!("Destination Genesis - {}", dh.hash());

		let horizon_header = src_chain.txhashset_archive_header().unwrap();

		debug!("Horizon header: {:?}", horizon_header);

		// Copy the headers from source to output in chunks
		let dest_sync_head = dest_chain.header_head().unwrap();
		let copy_chunk_size = 1000;
		let mut copied_header_index = 1;
		let mut src_headers = vec![];
		while copied_header_index <= horizon_header.height {
			let h = src_chain.get_header_by_height(copied_header_index).unwrap();
			src_headers.push(h);
			copied_header_index += 1;
			if copied_header_index % copy_chunk_size == 0 {
				debug!(
					"Copying headers to {} of {}",
					copied_header_index, horizon_header.height
				);
				dest_chain
					.sync_block_headers(&src_headers, dest_sync_head, Options::SKIP_POW)
					.unwrap();
				src_headers = vec![];
			}
		}
		if !src_headers.is_empty() {
			dest_chain
				.sync_block_headers(&src_headers, dest_sync_head, Options::NONE)
				.unwrap();
		}

		// Init segmenter, (note this still has to be lazy init somewhere on a peer)
		// This is going to use the same block as horizon_header
		let segmenter = src_chain.segmenter().unwrap();
		// Init desegmenter
		let mut desegmenter = dest_chain.desegmenter(&horizon_header).unwrap();

		// And total size of the bitmap PMMR
		let bitmap_mmr_size = desegmenter.expected_bitmap_mmr_size();
		debug!(
			"Bitmap Segments required: {}",
			SegmentIdentifier::count_segments_required(bitmap_mmr_size, target_segment_height)
		);
		// TODO: This can probably be derived from the PMMR we'll eventually be building
		// (check if total size is equal to total size at horizon header)
		let identifier_iter =
			SegmentIdentifier::traversal_iter(bitmap_mmr_size, target_segment_height);

		for sid in identifier_iter {
			debug!("Getting bitmap segment with Segment Identifier {:?}", sid);
			let (bitmap_segment, output_root_hash) = segmenter.bitmap_segment(sid).unwrap();
			debug!(
				"Bitmap segmenter reports output root hash is {:?}",
				output_root_hash
			);
			// Add segment to desegmenter / validate
			if let Err(e) = desegmenter.add_bitmap_segment(bitmap_segment, output_root_hash) {
				panic!("Unable to add bitmap segment: {}", e);
			}
		}

		// Finalize segmenter bitmap, which means we've recieved all bitmap MMR chunks and
		// Are ready to use it to validate outputs
		desegmenter.finalize_bitmap().unwrap();

		// OUTPUTS  - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			horizon_header.output_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			debug!("Getting output segment with Segment Identifier {:?}", sid);
			let (output_segment, bitmap_root_hash) = segmenter.output_segment(sid).unwrap();
			debug!(
				"Output segmenter reports bitmap hash is {:?}",
				bitmap_root_hash
			);
			// Add segment to desegmenter / validate
			if let Err(e) = desegmenter.add_output_segment(output_segment) {
				panic!("Unable to add output segment: {}", e);
			}
		}

		// PROOFS  - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			horizon_header.output_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			debug!(
				"Getting rangeproof segment with Segment Identifier {:?}",
				sid
			);
			let rangeproof_segment = segmenter.rangeproof_segment(sid).unwrap();
			// Add segment to desegmenter / validate
			if let Err(e) = desegmenter.add_rangeproof_segment(rangeproof_segment) {
				panic!("Unable to add rangeproof segment: {}", e);
			}
		}

		// KERNELS - Read + Validate
		let identifier_iter = SegmentIdentifier::traversal_iter(
			horizon_header.kernel_mmr_size,
			target_segment_height,
		);

		for sid in identifier_iter {
			debug!("Getting kernel segment with Segment Identifier {:?}", sid);
			let kernel_segment = segmenter.kernel_segment(sid).unwrap();
			if let Err(e) = desegmenter.add_kernel_segment(kernel_segment) {
				panic!("Unable to add kernel segment: {}", e);
			}
		}

		let dest_txhashset = dest_chain.txhashset();
		debug!("Dest TxHashset Roots: {:?}", dest_txhashset.read().roots());
	}
}

#[test]
fn test_pibd_copy_sample() {
	util::init_test_logger();
	// Note there is now a 'test' in grin_wallet_controller/build_chain
	// that can be manually tweaked to create a
	// small test chain with actual transaction data

	// Test on uncompacted and non-compacted chains
	let src_root_dir = format!("./chain/tests/test_data/chain_raw");
	let dest_root_dir = format!("./chain/tests/test_output/.segment_copy");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(true, &src_root_dir, &dest_root_dir);
	let src_root_dir = format!("./tests/test_data/chain_compacted");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(true, &src_root_dir, &dest_root_dir);
}

#[test]
#[ignore]
// As above, but run on a real instance of a chain pointed where you like
fn test_pibd_copy_real() {
	util::init_test_logger();
	// if testing against a real chain, insert location here
	let src_root_dir = format!("/Users/yeastplume/Projects/grin_project/server/chain_data");
	let dest_root_dir = format!("/Users/yeastplume/Projects/grin_project/server/.chain_data_copy");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(false, &src_root_dir, &dest_root_dir);
}
