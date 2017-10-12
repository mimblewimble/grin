// Copyright 2017 The Grin Developers
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

extern crate env_logger;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate rand;

use std::fs;

use chain::ChainStore;
use core::core::hash::Hashed;
use core::core::{Block, BlockHeader};
use keychain::Keychain;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_various_store_indices() {
	let _ = env_logger::init();
	clean_output_dir(".grin");

	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let chain_store = &chain::store::ChainKVStore::new(".grin".to_string()).unwrap() as &ChainStore;

	let block = Block::new(&BlockHeader::default(), vec![], &keychain, &key_id).unwrap();
	let commit = block.outputs[0].commitment();
	let block_hash = block.hash();

	chain_store.save_block(&block).unwrap();
	chain_store.setup_height(&block.header).unwrap();

	let block_header = chain_store.get_block_header(&block_hash).unwrap();
	assert_eq!(block_header.hash(), block_hash);

	let block_header = chain_store.get_header_by_height(1).unwrap();
	assert_eq!(block_header.hash(), block_hash);

	let block_header = chain_store
		.get_block_header_by_output_commit(&commit)
		.unwrap();
	assert_eq!(block_header.hash(), block_hash);
}
