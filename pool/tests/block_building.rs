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

extern crate chrono;
extern crate rand;

pub mod common;

use std::sync::{Arc, RwLock};

use core::core::{Block, BlockHeader};

use chain::txhashset;
use chain::types::Tip;
use core::core::hash::Hashed;
use core::core::target::Difficulty;

use keychain::{ExtKeychain, Keychain};
use wallet::libtx;

use common::*;

#[test]
fn test_transaction_pool_block_building() {
	util::init_test_logger();
	let keychain: ExtKeychain = Keychain::from_random_seed().unwrap();

	let db_root = ".grin_block_building".to_string();
	clean_output_dir(db_root.clone());
	let chain = ChainAdapter::init(db_root.clone()).unwrap();

	// Initialize the chain/txhashset with an initial block
	// so we have a non-empty UTXO set.
	let add_block = |height, txs| {
		let key_id = keychain.derive_key_id(height as u32).unwrap();
		let reward = libtx::reward::output(&keychain, &key_id, 0, height).unwrap();
		let mut block = Block::new(&BlockHeader::default(), txs, Difficulty::one(), reward).unwrap();

		let mut txhashset = chain.txhashset.write().unwrap();
		let mut batch = chain.store.batch().unwrap();
		txhashset::extending(&mut txhashset, &mut batch, |extension| {
			extension.apply_block(&block)?;

			// Now set the roots and sizes as necessary on the block header.
			let roots = extension.roots();
			block.header.output_root = roots.output_root;
			block.header.range_proof_root = roots.rproof_root;
			block.header.kernel_root = roots.kernel_root;
			let sizes = extension.sizes();
			block.header.output_mmr_size = sizes.0;
			block.header.kernel_mmr_size = sizes.2;

			Ok(())
		}).unwrap();

		let tip = Tip::from_block(&block.header);
		batch.save_block_header(&block.header).unwrap();
		batch.save_head(&tip).unwrap();
		batch.commit().unwrap();

		block.header
	};
	let header = add_block(1, vec![]);

	// Initialize a new pool with our chain adapter.
	let pool = RwLock::new(test_setup(&Arc::new(chain.clone())));

	// Now create tx to spend that first coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx = test_transaction_spending_coinbase(&keychain, &header, vec![10, 20, 30, 40]);

	// Mine that initial tx so we can spend it with multiple txs
	let header = add_block(2, vec![initial_tx]);

	let root_tx_1 = test_transaction(&keychain, vec![10, 20], vec![24]);
	let root_tx_2 = test_transaction(&keychain, vec![30], vec![28]);
	let root_tx_3 = test_transaction(&keychain, vec![40], vec![38]);

	let child_tx_1 = test_transaction(&keychain, vec![24], vec![22]);
	let child_tx_2 = test_transaction(&keychain, vec![38], vec![32]);

	{
		let mut write_pool = pool.write().unwrap();

		// Add the three root txs to the pool.
		write_pool
			.add_to_pool(test_source(), root_tx_1, false, &header.hash())
			.unwrap();
		write_pool
			.add_to_pool(test_source(), root_tx_2, false, &header.hash())
			.unwrap();
		write_pool
			.add_to_pool(test_source(), root_tx_3, false, &header.hash())
			.unwrap();

		// Now add the two child txs to the pool.
		write_pool
			.add_to_pool(test_source(), child_tx_1.clone(), false, &header.hash())
			.unwrap();
		write_pool
			.add_to_pool(test_source(), child_tx_2.clone(), false, &header.hash())
			.unwrap();

		assert_eq!(write_pool.total_size(), 5);
	}

	let txs = {
		let read_pool = pool.read().unwrap();
		read_pool.prepare_mineable_transactions()
	};
	// children should have been aggregated into parents
	assert_eq!(txs.len(), 3);

	let mut block = {
		let key_id = keychain.derive_key_id(2).unwrap();
		let fees = txs.iter().map(|tx| tx.fee()).sum();
		let reward = libtx::reward::output(&keychain, &key_id, fees, 0).unwrap();
		Block::new(&header, txs, Difficulty::one(), reward)
	}.unwrap();

	{
		let mut batch = chain.store.batch().unwrap();
		let mut txhashset = chain.txhashset.write().unwrap();
		txhashset::extending(&mut txhashset, &mut batch, |extension| {
			extension.apply_block(&block)?;

			// Now set the roots and sizes as necessary on the block header.
			let roots = extension.roots();
			block.header.output_root = roots.output_root;
			block.header.range_proof_root = roots.rproof_root;
			block.header.kernel_root = roots.kernel_root;
			let sizes = extension.sizes();
			block.header.output_mmr_size = sizes.0;
			block.header.kernel_mmr_size = sizes.2;

			Ok(())
		}).unwrap();

		let tip = Tip::from_block(&block.header);
		batch.save_block_header(&block.header).unwrap();
		batch.save_head(&tip).unwrap();
		batch.commit().unwrap();
	}

	// Now reconcile the transaction pool with the new block
	// and check the resulting contents of the pool are what we expect.
	{
		let mut write_pool = pool.write().unwrap();
		write_pool.reconcile_block(&block).unwrap();

		assert_eq!(write_pool.total_size(), 0);
	}
}
