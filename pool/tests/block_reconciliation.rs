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

use chain::ChainStore;
use chain::txhashset;
use chain::types::Tip;
use core::core::target::Difficulty;

use keychain::{ExtKeychain, Keychain};
use wallet::libtx;

use common::*;

#[test]
fn test_transaction_pool_block_reconciliation() {
	let keychain: ExtKeychain = Keychain::from_random_seed().unwrap();

	let db_root = ".grin_block_reconciliation".to_string();
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

	let header = {
		let key_id = keychain.derive_key_id(2).unwrap();
		let fees = initial_tx.fee();
		let reward = libtx::reward::output(&keychain, &key_id, fees, 0).unwrap();
		let block = Block::new(&header, vec![initial_tx], Difficulty::one(), reward).unwrap();

		{
			let mut txhashset = chain.txhashset.write().unwrap();
			txhashset::extending(&mut txhashset, |extension| {
				extension.apply_block(&block)?;
				Ok(())
			}).unwrap();
		}

		let tip = Tip::from_block(&block.header);
		chain.store.save_block_header(&block.header).unwrap();
		chain.store.save_head(&tip).unwrap();

		block.header
	};

	// Preparation: We will introduce three root pool transactions.
	// 1. A transaction that should be invalidated because it is exactly
	//  contained in the block.
	// 2. A transaction that should be invalidated because the input is
	//  consumed in the block, although it is not exactly consumed.
	// 3. A transaction that should remain after block reconciliation.
	let block_transaction = test_transaction(&keychain, vec![10], vec![8]);
	let conflict_transaction = test_transaction(&keychain, vec![20], vec![12, 6]);
	let valid_transaction = test_transaction(&keychain, vec![30], vec![13, 15]);

	// We will also introduce a few children:
	// 4. A transaction that descends from transaction 1, that is in
	//  turn exactly contained in the block.
	let block_child = test_transaction(&keychain, vec![8], vec![5, 1]);
	// 5. A transaction that descends from transaction 4, that is not
	//  contained in the block at all and should be valid after
	//  reconciliation.
	let pool_child = test_transaction(&keychain, vec![5], vec![3]);
	// 6. A transaction that descends from transaction 2 that does not
	//  conflict with anything in the block in any way, but should be
	//  invalidated (orphaned).
	let conflict_child = test_transaction(&keychain, vec![12], vec![2]);
	// 7. A transaction that descends from transaction 2 that should be
	//  valid due to its inputs being satisfied by the block.
	let conflict_valid_child = test_transaction(&keychain, vec![6], vec![4]);
	// 8. A transaction that descends from transaction 3 that should be
	//  invalidated due to an output conflict.
	let valid_child_conflict = test_transaction(&keychain, vec![13], vec![9]);
	// 9. A transaction that descends from transaction 3 that should remain
	//  valid after reconciliation.
	let valid_child_valid = test_transaction(&keychain, vec![15], vec![11]);
	// 10. A transaction that descends from both transaction 6 and
	//  transaction 9
	let mixed_child = test_transaction(&keychain, vec![2, 11], vec![7]);

	let txs_to_add = vec![
		block_transaction,
		conflict_transaction,
		valid_transaction.clone(),
		block_child,
		pool_child.clone(),
		conflict_child,
		conflict_valid_child.clone(),
		valid_child_conflict.clone(),
		valid_child_valid.clone(),
		mixed_child,
	];

	// First we add the above transactions to the pool.
	// All should be accepted.
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		for tx in &txs_to_add {
			write_pool
				.add_to_pool(test_source(), tx.clone(), false)
				.unwrap();
		}

		assert_eq!(write_pool.total_size(), txs_to_add.len());
	}

	// Now we prepare the block that will cause the above conditions to be met.
	// First, the transactions we want in the block:
	// - Copy of 1
	let block_tx_1 = test_transaction(&keychain, vec![10], vec![8]);
	// - Conflict w/ 2, satisfies 7
	let block_tx_2 = test_transaction(&keychain, vec![20], vec![6]);
	// - Copy of 4
	let block_tx_3 = test_transaction(&keychain, vec![8], vec![5, 1]);
	// - Output conflict w/ 8
	let block_tx_4 = test_transaction(&keychain, vec![40], vec![9, 31]);
	let block_txs = vec![block_tx_1, block_tx_2, block_tx_3, block_tx_4];

	// Now apply this block.
	let block = {
		let key_id = keychain.derive_key_id(3).unwrap();
		let fees = block_txs.iter().map(|tx| tx.fee()).sum();
		let reward = libtx::reward::output(&keychain, &key_id, fees, 0).unwrap();
		let block = Block::new(&header, block_txs, Difficulty::one(), reward).unwrap();

		{
			let mut txhashset = chain.txhashset.write().unwrap();
			txhashset::extending(&mut txhashset, |extension| {
				extension.apply_block(&block)?;
				Ok(())
			}).unwrap();
		}

		let tip = Tip::from_block(&block.header);
		chain.store.save_block_header(&block.header).unwrap();
		chain.store.save_head(&tip).unwrap();

		block
	};

	// And reconcile the pool with this latest block.
	{
		let mut write_pool = pool.write().unwrap();
		write_pool.reconcile_block(&block).unwrap();

		// TODO - this is the "correct" behavior (see below)
		// assert_eq!(write_pool.total_size(), 4);
		// assert_eq!(write_pool.txpool.entries[0].tx, valid_transaction);
		// assert_eq!(write_pool.txpool.entries[1].tx, pool_child);
		// assert_eq!(write_pool.txpool.entries[2].tx, conflict_valid_child);
		// assert_eq!(write_pool.txpool.entries[3].tx, valid_child_valid);

		//
		// TODO - once the hash() vs hash_with_index(pos - 1) change is made in
		// txhashset.apply_output() TODO - and we no longer incorrectly allow
		// duplicate outputs in the MMR TODO - then this test will fail
		//
		// TODO - wtf is with these name permutations...
		//
		assert_eq!(write_pool.total_size(), 4);
		assert_eq!(write_pool.txpool.entries[0].tx, valid_transaction);
		assert_eq!(write_pool.txpool.entries[1].tx, conflict_valid_child);
		assert_eq!(write_pool.txpool.entries[2].tx, valid_child_conflict);
		assert_eq!(write_pool.txpool.entries[3].tx, valid_child_valid);
	}
}
