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
extern crate grin_store as store;
extern crate grin_wallet as wallet;
extern crate rand;

use std::fs;
use std::sync::Arc;

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
	match env_logger::try_init() {
		Ok(_) => println!("Initializing env logger"),
		Err(e) => println!("env logger already initialized: {:?}", e),
	};
	let chain_dir = ".grin_idx_1";
	clean_output_dir(chain_dir);

	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();
	let db_env = Arc::new(store::new_env(chain_dir.to_string()));

	let chain_store = chain::store::ChainStore::new(db_env).unwrap();
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis = pow::mine_genesis_block().unwrap();
	let reward = libtx::reward::output(&keychain, &key_id, 0, 1).unwrap();

	let block = Block::new(&genesis.header, vec![], Difficulty::one(), reward).unwrap();
	let block_hash = block.hash();

	{
		let batch = chain_store.batch().unwrap();
		batch.save_block(&genesis).unwrap();
		batch
			.setup_height(&genesis.header, &Tip::new(genesis.hash()))
			.unwrap();
		batch.commit().unwrap();
	}
	{
		let batch = chain_store.batch().unwrap();
		batch.save_block(&block).unwrap();
		batch
			.setup_height(&block.header, &Tip::from_block(&block.header))
			.unwrap();

		batch.commit().unwrap();
	}

	let block_header = chain_store.get_block_header(&block_hash).unwrap();
	assert_eq!(block_header.hash(), block_hash);

	let block_header = chain_store.get_header_by_height(1).unwrap();
	assert_eq!(block_header.hash(), block_hash);
}

#[test]
fn test_store_header_height() {
	match env_logger::try_init() {
		Ok(_) => println!("Initializing env logger"),
		Err(e) => println!("env logger already initialized: {:?}", e),
	};
	let chain_dir = ".grin_idx_2";
	clean_output_dir(chain_dir);

	let db_env = Arc::new(store::new_env(chain_dir.to_string()));
	let chain_store = chain::store::ChainStore::new(db_env).unwrap();

	let mut block_header = BlockHeader::default();
	block_header.height = 1;

	{
		let batch = chain_store.batch().unwrap();
		batch.save_block_header(&block_header).unwrap();
		batch.save_header_height(&block_header).unwrap();
		batch.commit().unwrap();
	}

	let stored_block_header = chain_store.get_header_by_height(1).unwrap();
	assert_eq!(block_header.hash(), stored_block_header.hash());

	{
		let batch = chain_store.batch().unwrap();
		batch.delete_header_by_height(1).unwrap();
		batch.commit().unwrap();
	}

	let result = chain_store.get_header_by_height(1);
	assert_eq!(result.is_err(), true);
}
