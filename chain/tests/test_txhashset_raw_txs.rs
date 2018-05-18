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

extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_wallet as wallet;

use std::fs;
use std::sync::Arc;

use chain::txhashset;
use chain::txhashset::TxHashSet;
use chain::types::Tip;
use chain::ChainStore;
use chain::store::ChainKVStore;
use core::core::{Block, BlockHeader};
use core::core::target::Difficulty;
use core::core::pmmr::MerkleProof;
use keychain::{Identifier, Keychain};
use wallet::libwallet::{build, reward};


fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}


#[test]
fn test_some_raw_txs() {
	let db_root = format!(".grin_txhashset_raw_txs");
	clean_output_dir(&db_root);

	let store = Arc::new(ChainKVStore::new(db_root.clone()).unwrap());
	let mut txhashset = TxHashSet::open(db_root.clone(), store.clone()).unwrap();

	let keychain = Keychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();
	let key_id4 = keychain.derive_key_id(4).unwrap();

	// // Create a simple block with a single coinbase output so we have something to spend.
	let prev_header = BlockHeader::default();
	let reward_output = reward::output(&keychain, &key_id1, 0, prev_header.height).unwrap();
	let block = Block::new(&prev_header, vec![], Difficulty::one(), reward_output).unwrap();

	// Apply this block to the txhashset to give us a non-empty starting point.
	txhashset::extending(&mut txhashset, |extension| {
		extension.apply_block(&block)
	}).unwrap();

	// Make sure we setup the head in the store based on block we just accepted.
	let head = Tip::from_block(&block.header);
	store.save_head(&head).unwrap();

	// We will use the "next" block height when applying the raw tx to the txhashset
	let mut height = block.header.height + 1;

	let coinbase_reward = 60_000_000_000;

	// tx1 spends the original coinbase output from the block
	let tx1 = build::transaction_with_offset(
			vec![
				build::coinbase_input(coinbase_reward, block.hash(), MerkleProof::default(), key_id1.clone()),
				build::output(100, key_id2.clone()),
			],
			&keychain,
		).unwrap();

	// tx2 attempts to "double spend" the coinbase output from the block (conflicts with tx1)
	let tx2 = build::transaction_with_offset(
			vec![
				build::coinbase_input(coinbase_reward, block.hash(), MerkleProof::default(), key_id1.clone()),
				build::output(100, key_id3.clone()),
			],
			&keychain,
		).unwrap();

	// tx3 spends the output from tx1
	let tx3 = build::transaction_with_offset(
			vec![
				build::input(100, key_id2.clone()),
				build::output(98, key_id4.clone()),
			],
			&keychain,
		).unwrap();

	// tx1 is valid
	// tx2 should be invalid (based on conflict with tx1)
	// tx3 should also be valid
	let txs = vec![tx1, tx2, tx3];

	// Now validate the txs against the txhashset (via a readonly extension)
	txhashset::extending_readonly(&mut txhashset, |extension| {
		for tx in txs {
			height += 1;
			let res = extension.apply_raw_tx(&tx, height);
			println!("***** raw_tx in the loop {:?}", res);
		}
		Ok(())
	}).unwrap();

	panic!("...");
}
