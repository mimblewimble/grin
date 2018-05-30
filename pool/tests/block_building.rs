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

extern crate blake2_rfc as blake2;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_pool as pool;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate rand;
extern crate time;

pub mod common;

use std::sync::{Arc, RwLock};

use core::core::{Block, BlockHeader};

use chain::txhashset;
use chain::types::Tip;
use chain::ChainStore;
use core::core::target::Difficulty;

use keychain::Keychain;
use wallet::libtx;

use common::*;

#[test]
fn test_transaction_pool_block_building() {
	let keychain = Keychain::from_random_seed().unwrap();

	let db_root = ".grin_block_building".to_string();
	clean_output_dir(db_root.clone());
	let chain = ChainAdapter::init(db_root.clone()).unwrap();

	// Initialize the chain/txhashset with an initial block
	// so we have a non-empty UTXO set.
	let header = {
		let height = 1;
		let key_id = keychain.derive_key_id(height as u32).unwrap();
		let reward = libtx::reward::output(&keychain, &key_id, 0, height).unwrap();
		let block = Block::new(&BlockHeader::default(), vec![], Difficulty::one(), reward).unwrap();

		let mut txhashset = chain.txhashset.write().unwrap();
		txhashset::extending(&mut txhashset, |extension| extension.apply_block(&block)).unwrap();

		let tip = Tip::from_block(&block.header);
		chain.store.save_block_header(&block.header).unwrap();
		chain.store.save_head(&tip).unwrap();

		block.header
	};

	// Initialize a new pool with our chain adapter.
	let pool = RwLock::new(test_setup(&Arc::new(chain.clone())));

	// Now create tx to spend that first coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx = test_transaction_spending_coinbase(&keychain, &header, vec![10, 20, 30, 40]);

	// Add this tx to the pool (stem=false, direct to txpool).
	{
		let mut write_pool = pool.write().unwrap();
		write_pool
			.add_to_pool(test_source(), initial_tx, false)
			.unwrap();
		assert_eq!(write_pool.total_size(), 1);
	}

	let root_tx_1 = test_transaction(&keychain, vec![10, 20], vec![24]);
	let root_tx_2 = test_transaction(&keychain, vec![30], vec![28]);
	let root_tx_3 = test_transaction(&keychain, vec![40], vec![38]);

	let child_tx_1 = test_transaction(&keychain, vec![24], vec![22]);
	let child_tx_2 = test_transaction(&keychain, vec![38], vec![32]);

	{
		let mut write_pool = pool.write().unwrap();

		// Add the three root txs to the pool.
		write_pool
			.add_to_pool(test_source(), root_tx_1, false)
			.unwrap();
		write_pool
			.add_to_pool(test_source(), root_tx_2, false)
			.unwrap();
		write_pool
			.add_to_pool(test_source(), root_tx_3, false)
			.unwrap();

		// Now add the two child txs to the pool.
		write_pool
			.add_to_pool(test_source(), child_tx_1.clone(), false)
			.unwrap();
		write_pool
			.add_to_pool(test_source(), child_tx_2.clone(), false)
			.unwrap();

		assert_eq!(write_pool.total_size(), 6);
	}

	let txs = {
		let read_pool = pool.read().unwrap();
		read_pool.prepare_mineable_transactions(4)
	};
	assert_eq!(txs.len(), 4);

	let block = {
		let key_id = keychain.derive_key_id(2).unwrap();
		let fees = txs.iter().map(|tx| tx.fee()).sum();
		let reward = libtx::reward::output(&keychain, &key_id, fees, 0).unwrap();
		Block::new(&header, txs, Difficulty::one(), reward)
	}.unwrap();

	{
		let mut txhashset = chain.txhashset.write().unwrap();
		txhashset::extending(&mut txhashset, |extension| {
			extension.apply_block(&block)?;
			Ok(())
		}).unwrap();
	}

	// Now reconcile the transaction pool with the new block
	// and check the resulting contents of the pool are what we expect.
	{
		let mut write_pool = pool.write().unwrap();
		write_pool.reconcile_block(&block).unwrap();

		assert_eq!(write_pool.total_size(), 2);
		assert_eq!(write_pool.txpool.entries[0].tx, child_tx_1);
		assert_eq!(write_pool.txpool.entries[1].tx, child_tx_2);
	}
}
