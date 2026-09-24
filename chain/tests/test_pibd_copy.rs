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

use crate::chain::txhashset::BitmapChunk;
use crate::chain::types::{NoopAdapter, Options};
use crate::core::core::{
	hash::{Hash, Hashed},
	pmmr::segment::{Segment, SegmentIdentifier, SegmentType},
	Block, OutputIdentifier, TxKernel,
};
use crate::core::{genesis, global, pow};
use crate::util::secp::pedersen::RangeProof;

use self::chain_test_helper::clean_output_dir;

mod chain_test_helper;

// Canned segmenter responder, which will simulate feeding back segments as requested
// by the desegmenter
struct SegmenterResponder {
	chain: Arc<chain::Chain>,
}

impl SegmenterResponder {
	pub fn new(chain_src_dir: &str, genesis: Block) -> Self {
		let dummy_adapter = Arc::new(NoopAdapter {});
		debug!(
			"Reading SegmenterResponder chain, genesis block: {}",
			genesis.hash()
		);

		// The original chain we're reading from
		let res = SegmenterResponder {
			chain: Arc::new(
				chain::Chain::init(
					chain_src_dir.into(),
					dummy_adapter.clone(),
					genesis,
					pow::verify_size,
					false,
					None,
				)
				.unwrap(),
			),
		};
		let sh = res.chain.get_header_by_height(0).unwrap();
		debug!("Source Genesis - {}", sh.hash());
		res
	}

	pub fn chain(&self) -> Arc<chain::Chain> {
		self.chain.clone()
	}

	pub fn get_bitmap_segment(&self, seg_id: SegmentIdentifier) -> (Segment<BitmapChunk>, Hash) {
		let segmenter = self.chain.segmenter().unwrap();
		segmenter.bitmap_segment(seg_id).unwrap()
	}

	pub fn get_output_segment(
		&self,
		seg_id: SegmentIdentifier,
	) -> (Segment<OutputIdentifier>, Hash) {
		let segmenter = self.chain.segmenter().unwrap();
		segmenter.output_segment(seg_id).unwrap()
	}

	pub fn get_rangeproof_segment(&self, seg_id: SegmentIdentifier) -> Segment<RangeProof> {
		let segmenter = self.chain.segmenter().unwrap();
		segmenter.rangeproof_segment(seg_id).unwrap()
	}

	pub fn get_kernel_segment(&self, seg_id: SegmentIdentifier) -> Segment<TxKernel> {
		let segmenter = self.chain.segmenter().unwrap();
		segmenter.kernel_segment(seg_id).unwrap()
	}
}

// Canned segmenter 'peer', building up its local chain from requested PIBD segments
struct DesegmenterRequestor {
	chain: Arc<chain::Chain>,
	responder: Arc<SegmenterResponder>,
}

impl DesegmenterRequestor {
	pub fn new(chain_src_dir: &str, genesis: Block, responder: Arc<SegmenterResponder>) -> Self {
		let dummy_adapter = Arc::new(NoopAdapter {});
		debug!(
			"Reading DesegmenterRequestor chain, genesis block: {}",
			genesis.hash()
		);

		// The original chain we're reading from
		let res = DesegmenterRequestor {
			chain: Arc::new(
				chain::Chain::init(
					chain_src_dir.into(),
					dummy_adapter.clone(),
					genesis,
					pow::verify_size,
					false,
					None,
				)
				.unwrap(),
			),
			responder,
		};
		let sh = res.chain.get_header_by_height(0).unwrap();
		debug!("Dest Genesis - {}", sh.hash());
		res
	}

	/// Copy headers, hopefully bringing the requestor to a state where PIBD is the next step
	pub fn copy_headers_from_responder(&mut self) {
		let src_chain = self.responder.chain();
		let tip = src_chain.header_head().unwrap();
		let dest_sync_head = self.chain.header_head().unwrap();
		// Keep test writes small enough for their 1 MB LMDB allocation
		let copy_chunk_size = if global::is_production_mode() {
			1000
		} else {
			10
		};
		let mut copied_header_index = 1;
		let mut src_headers = vec![];
		while copied_header_index <= tip.height {
			let h = src_chain.get_header_by_height(copied_header_index).unwrap();
			src_headers.push(h);
			copied_header_index += 1;
			if copied_header_index % copy_chunk_size == 0 {
				debug!(
					"Copying headers to {} of {}",
					copied_header_index, tip.height
				);
				self.chain
					.sync_block_headers(&src_headers, dest_sync_head, Options::SKIP_POW)
					.unwrap();
				src_headers = vec![];
			}
		}
		if !src_headers.is_empty() {
			self.chain
				.sync_block_headers(&src_headers, dest_sync_head, Options::NONE)
				.unwrap();
		}
	}

	// Emulate `continue_pibd` function, which would be called from state sync
	// return whether is complete
	pub fn continue_pibd(&mut self) -> bool {
		let archive_header = self.chain.txhashset_archive_header_header_only().unwrap();
		let desegmenter = self.chain.desegmenter(&archive_header).unwrap();

		// Apply segments... TODO: figure out how this should be called, might
		// need to be a separate thread.
		if let Some(mut de) = desegmenter.try_write() {
			if let Some(d) = de.as_mut() {
				d.apply_next_segments().unwrap();
			}
		}

		let mut next_segment_ids = vec![];
		let mut is_complete = false;
		if let Some(d) = desegmenter.write().as_mut() {
			// Figure out the next segments we need
			// (12 is divisible by 3, to try and evenly spread the requests among the 3
			// main pmmrs. Bitmaps segments will always be requested first)
			next_segment_ids = d.next_desired_segments(12);
			is_complete = d.is_complete()
		}

		debug!("Next segment IDS: {:?}", next_segment_ids);

		// For each segment, pick a desirable peer and send message
		for seg_id in next_segment_ids.iter() {
			// Perform request and response
			match seg_id.segment_type {
				SegmentType::Bitmap => {
					let (seg, output_root) =
						self.responder.get_bitmap_segment(seg_id.identifier.clone());
					if let Some(d) = desegmenter.write().as_mut() {
						d.add_bitmap_segment(seg, output_root).unwrap();
					}
				}
				SegmentType::Output => {
					let (seg, _bitmap_root) =
						self.responder.get_output_segment(seg_id.identifier.clone());
					if let Some(d) = desegmenter.write().as_mut() {
						d.add_output_segment(seg).unwrap();
					}
				}
				SegmentType::RangeProof => {
					let seg = self
						.responder
						.get_rangeproof_segment(seg_id.identifier.clone());
					if let Some(d) = desegmenter.write().as_mut() {
						d.add_rangeproof_segment(seg).unwrap();
					}
				}
				SegmentType::Kernel => {
					let seg = self.responder.get_kernel_segment(seg_id.identifier.clone());
					if let Some(d) = desegmenter.write().as_mut() {
						d.add_kernel_segment(seg).unwrap();
					}
				}
			};
		}
		is_complete
	}

	pub fn check_roots(&self) {
		let roots = self.chain.txhashset().read().roots().unwrap();
		let archive_header = self.chain.txhashset_archive_header_header_only().unwrap();
		debug!("Archive Header is {:?}", archive_header);
		debug!("TXHashset output root is {:?}", roots);
		debug!(
			"TXHashset merged output root is {:?}",
			roots.output_roots.root(&archive_header)
		);
		assert_eq!(archive_header.range_proof_root, roots.rproof_root);
		assert_eq!(archive_header.kernel_root, roots.kernel_root);
		assert_eq!(
			archive_header.output_root,
			roots.output_roots.root(&archive_header)
		);
	}
}
fn test_pibd_copy_impl(is_fixture: bool, src_root_dir: &str, dest_root_dir: &str) {
	global::set_local_chain_type(global::ChainTypes::Testnet);
	global::set_local_nrd_enabled(true);
	let mut genesis = genesis::genesis_test();

	if is_fixture {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
		genesis = pow::mine_genesis_block().unwrap();
	}

	let src_responder = Arc::new(SegmenterResponder::new(src_root_dir, genesis.clone()));
	let mut dest_requestor =
		DesegmenterRequestor::new(dest_root_dir, genesis.clone(), src_responder);

	dest_requestor.copy_headers_from_responder();

	// Perform until desegmenter reports it's done
	while !dest_requestor.continue_pibd() {}

	dest_requestor.check_roots();
}

#[test]
fn test_pibd_copy_sample() {
	util::init_test_logger();
	// Rebuild both fixtures via PIBD and compare their roots
	let src_root_dir = format!("./tests/test_data/chain_raw");
	let dest_root_dir = format!("./tests/test_output/.segment_copy");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(true, &src_root_dir, &dest_root_dir);
	let src_root_dir = format!("./tests/test_data/chain_compacted");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(true, &src_root_dir, &dest_root_dir);
	clean_output_dir(&dest_root_dir);
}

#[test]
#[ignore]
// Run with --ignored and set GRIN_CHAIN_DATA to a synced testnet chain_data directory
fn test_pibd_copy_real() {
	util::init_test_logger();
	let src_root_dir = std::env::var("GRIN_CHAIN_DATA").expect("GRIN_CHAIN_DATA must be set");
	let dest_root_dir = format!("./tests/test_output/.segment_copy_real");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(false, &src_root_dir, &dest_root_dir);
	clean_output_dir(&dest_root_dir);
}
