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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};

use crate::chain::txhashset::BitmapChunk;
use crate::chain::types::{NoopAdapter, Options};
use crate::core::core::{
	hash::{Hash, Hashed},
	pmmr::segment::{Segment, SegmentIdentifier, SegmentType},
	Block, OutputIdentifier,
};
use crate::core::{genesis, global, pow};

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
	pub fn continue_pibd(&mut self) {
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
		if let Some(d) = desegmenter.write().as_mut() {
			// Figure out the next segments we need
			// (12 is divisible by 3, to try and evenly spread the requests among the 3
			// main pmmrs. Bitmaps segments will always be requested first)
			next_segment_ids = d.next_desired_segments(12);
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
				_ => {} /*SegmentType::RangeProof => p
							.send_rangeproof_segment_request(
								archive_header.hash(),
								seg_id.identifier.clone(),
							)
							.unwrap(),
						SegmentType::Kernel => p
							.send_kernel_segment_request(
								archive_header.hash(),
								seg_id.identifier.clone(),
							)
							.unwrap(),*/
			};
		}
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
	}
}
fn test_pibd_copy_impl(
	is_test_chain: bool,
	src_root_dir: &str,
	dest_root_dir: &str,
	dest_template_dir: Option<&str>,
) {
	global::set_local_chain_type(global::ChainTypes::Mainnet);
	let mut genesis = genesis::genesis_main();

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

	{
		let src_responder = Arc::new(SegmenterResponder::new(src_root_dir, genesis.clone()));
		let mut dest_requestor =
			DesegmenterRequestor::new(dest_root_dir, genesis.clone(), src_responder);

		// No template provided so copy headers from source
		if dest_template_dir.is_none() {
			dest_requestor.copy_headers_from_responder();
			return;
		}

		// Just peform a set number of times for now
		for _ in 0..10000 {
			dest_requestor.continue_pibd();
		}

		dest_requestor.check_roots();

		/*let horizon_header = src_chain.txhashset_archive_header().unwrap();

		debug!("Horizon header: {:?}", horizon_header);*/

		// Copy the headers from source to output in chunks
		/*let dest_sync_head = dest_chain.header_head().unwrap();
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
		}*/

		// Init segmenter, (note this still has to be lazy init somewhere on a peer)
		// This is going to use the same block as horizon_header
		/*klet segmenter = src_chain.segmenter().unwrap();
		// Init desegmenter
		let desegmenter_lock = dest_chain.desegmenter(&horizon_header).unwrap();
		let mut desegmenter_write = desegmenter_lock.write();
		let desegmenter = desegmenter_write.as_mut().unwrap();

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
			if let Err(e) = desegmenter.apply_next_segments() {
				panic!("Unable to apply bitmap segment: {}", e);
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
			if let Err(e) = desegmenter.add_output_segment(output_segment, None) {
				panic!("Unable to add output segment: {}", e);
			}
			if let Err(e) = desegmenter.apply_next_segments() {
				panic!("Unable to apply output segment: {}", e);
			}
		}*/

		// PROOFS  - Read + Validate
		/*let identifier_iter = SegmentIdentifier::traversal_iter(
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
		*/
	}
}

#[test]
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
//#[ignore]
// Note this test is intended to be run manually, as testing the copy of an
// entire live chain is beyond the capability of current CI
// As above, but run on a real instance of a chain pointed where you like
fn test_pibd_copy_real() {
	util::init_test_logger();
	// If set, just copy headers from source to target template dir and exit
	// Used to set up a chain state simulating the start of PIBD to continue manual testing
	let copy_headers_to_template = false;

	// if testing against a real chain, insert location here
	let src_root_dir = format!("/home/yeastplume/Projects/grin-project/servers/sync-1/chain_data");
	let dest_template_dir =
		format!("/home/yeastplume/Projects/grin-project/servers/sync-1/chain_data_headers_applied");
	let dest_root_dir =
		format!("/home/yeastplume/Projects/grin-project/servers/sync-1/chain_data_copy");
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
