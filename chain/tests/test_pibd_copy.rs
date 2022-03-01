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

use std::path::Path;
use std::sync::Arc;
use std::{fs, io};

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

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
	fs::create_dir_all(&dst)?;
	for entry in fs::read_dir(src)? {
		let entry = entry?;
		let ty = entry.file_type()?;
		if ty.is_dir() {
			copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
		} else {
			fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
		}
	}
	Ok(())
}

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
		let copy_chunk_size = 1000;
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
					let (seg, bitmap_root) =
						self.responder.get_output_segment(seg_id.identifier.clone());
					if let Some(d) = desegmenter.write().as_mut() {
						d.add_output_segment(seg, Some(bitmap_root)).unwrap();
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
		let roots = self.chain.txhashset().read().roots();
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
fn test_pibd_copy_impl(
	is_test_chain: bool,
	src_root_dir: &str,
	dest_root_dir: &str,
	dest_template_dir: Option<&str>,
) {
	global::set_local_chain_type(global::ChainTypes::Testnet);
	let mut genesis = genesis::genesis_test();

	if is_test_chain {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
		genesis = pow::mine_genesis_block().unwrap();
	}

	// Copy a starting point over for the destination, e.g. a copy of chain
	// with all headers pre-applied
	if let Some(td) = dest_template_dir {
		debug!(
			"Copying template dir for destination from {} to {}",
			td, dest_root_dir
		);
		copy_dir_all(td, dest_root_dir).unwrap();
	}

	let src_responder = Arc::new(SegmenterResponder::new(src_root_dir, genesis.clone()));
	let mut dest_requestor =
		DesegmenterRequestor::new(dest_root_dir, genesis.clone(), src_responder);

	// No template provided so copy headers from source
	if dest_template_dir.is_none() {
		dest_requestor.copy_headers_from_responder();
		if !is_test_chain {
			return;
		}
	}

	// Perform until desegmenter reports it's done
	while !dest_requestor.continue_pibd() {}

	dest_requestor.check_roots();
}

#[test]
#[ignore]
fn test_pibd_copy_sample() {
	util::init_test_logger();
	// Note there is now a 'test' in grin_wallet_controller/build_chain
	// that can be manually tweaked to create a
	// small test chain with actual transaction data

	// Test on uncompacted and non-compacted chains
	let src_root_dir = format!("./tests/test_data/chain_raw");
	let dest_root_dir = format!("./tests/test_output/.segment_copy");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(true, &src_root_dir, &dest_root_dir, None);
	let src_root_dir = format!("./tests/test_data/chain_compacted");
	clean_output_dir(&dest_root_dir);
	test_pibd_copy_impl(true, &src_root_dir, &dest_root_dir, None);
	clean_output_dir(&dest_root_dir);
}

#[test]
#[ignore]
// Note this test is intended to be run manually, as testing the copy of an
// entire live chain is beyond the capability of current CI
// As above, but run on a real instance of a chain pointed where you like
fn test_pibd_copy_real() {
	util::init_test_logger();
	// If set, just copy headers from source to target template dir and exit
	// Used to set up a chain state simulating the start of PIBD to continue manual testing
	let copy_headers_to_template = false;

	// if testing against a real chain, insert location here
	let src_root_dir = format!("/home/yeastplume/Projects/grin-project/servers/floo-1/chain_data");
	let dest_template_dir = format!(
		"/home/yeastplume/Projects/grin-project/servers/floo-pibd-1/chain_data_headers_only"
	);
	let dest_root_dir =
		format!("/home/yeastplume/Projects/grin-project/servers/floo-pibd-1/chain_data_test_copy");
	if copy_headers_to_template {
		clean_output_dir(&dest_template_dir);
		test_pibd_copy_impl(false, &src_root_dir, &dest_template_dir, None);
	} else {
		clean_output_dir(&dest_root_dir);
		test_pibd_copy_impl(
			false,
			&src_root_dir,
			&dest_root_dir,
			Some(&dest_template_dir),
		);
	}

	//clean_output_dir(&dest_root_dir);
}
