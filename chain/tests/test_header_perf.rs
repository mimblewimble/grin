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
use crate::core::core::hash::Hashed;
use crate::core::{genesis, global, pow};

use self::chain_test_helper::clean_output_dir;

mod chain_test_helper;

fn test_header_perf_impl(is_test_chain: bool, src_root_dir: &str, dest_root_dir: &str) {
	global::set_local_chain_type(global::ChainTypes::Mainnet);
	let mut genesis = genesis::genesis_main();

	if is_test_chain {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
		genesis = pow::mine_genesis_block().unwrap();
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
		while copied_header_index <= 100000 {
			let h = src_chain.get_header_by_height(copied_header_index).unwrap();
			src_headers.push(h);
			copied_header_index += 1;
			if copied_header_index % copy_chunk_size == 0 {
				debug!(
					"Copying headers to {} of {}",
					copied_header_index, horizon_header.height
				);
				dest_chain
					.sync_block_headers(&src_headers, dest_sync_head, Options::NONE)
					.unwrap();
				src_headers = vec![];
			}
		}
		if !src_headers.is_empty() {
			dest_chain
				.sync_block_headers(&src_headers, dest_sync_head, Options::NONE)
				.unwrap();
		}
	}
}

#[test]
#[ignore]
// Ignored during CI, but use this to run this test on a real instance of a chain pointed where you like
fn test_header_perf() {
	util::init_test_logger();
	// if testing against a real chain, insert location here
	// NOTE: Modify to point at your own paths
	let src_root_dir = format!("/Users/yeastplume/Projects/grin_project/server/chain_data");
	let dest_root_dir = format!("/Users/yeastplume/Projects/grin_project/server/.chain_data_copy");
	clean_output_dir(&dest_root_dir);
	test_header_perf_impl(false, &src_root_dir, &dest_root_dir);
	clean_output_dir(&dest_root_dir);
}
