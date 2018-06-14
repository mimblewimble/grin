// Copyright 2018 The Grin Developers
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
extern crate grin_wallet as wallet;
extern crate rand;

use std::fs;

use chain::{ChainStore, Tip};
use core::core::hash::Hashed;
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader};
use core::global::{self, ChainTypes};
use core::pow;
use keychain::{ExtKeychain, Keychain};
use wallet::libtx;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_various_store_indices() {
	let _ = env_logger::init();
	let chain_dir = ".grin_idx_1";
	clean_output_dir(chain_dir);

	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let chain_store =
		&chain::store::ChainKVStore::new(chain_dir.to_string()).unwrap() as &ChainStore;

	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis = pow::mine_genesis_block().unwrap();
	chain_store.save_block(&genesis).unwrap();
	chain_store
		.setup_height(&genesis.header, &Tip::new(genesis.hash()))
		.unwrap();

	let reward = libtx::reward::output(&keychain, &key_id, 0, 1).unwrap();

	let block = Block::new(&genesis.header, vec![], Difficulty::one(), reward).unwrap();
	let block_hash = block.hash();

	chain_store.save_block(&block).unwrap();
	chain_store
		.setup_height(&block.header, &Tip::from_block(&block.header))
		.unwrap();

	let block_header = chain_store.get_block_header(&block_hash).unwrap();
	assert_eq!(block_header.hash(), block_hash);

	let block_header = chain_store.get_header_by_height(1).unwrap();
	assert_eq!(block_header.hash(), block_hash);
}

#[test]
fn test_store_header_height() {
	let _ = env_logger::init();
	let chain_dir = ".grin_idx_2";
	clean_output_dir(chain_dir);

	let chain_store =
		&chain::store::ChainKVStore::new(chain_dir.to_string()).unwrap() as &ChainStore;

	let mut block_header = BlockHeader::default();
	block_header.height = 1;

	chain_store.save_block_header(&block_header).unwrap();
	chain_store.save_header_height(&block_header).unwrap();

	let stored_block_header = chain_store.get_header_by_height(1).unwrap();
	assert_eq!(block_header.hash(), stored_block_header.hash());

	chain_store.delete_header_by_height(1).unwrap();

	let result = chain_store.get_header_by_height(1);
	assert_eq!(result.is_err(), true);
}
