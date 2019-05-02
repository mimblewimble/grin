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

//! Test coverage for block building at the limit of max_block_weight.

pub mod common;

use self::core::core::hash::Hashed;
use self::core::core::verifier_cache::LruVerifierCache;
use self::core::core::{Block, BlockHeader, Transaction};
use self::core::global;
use self::core::libtx;
use self::core::pow::Difficulty;
use self::keychain::{ExtKeychain, Keychain};
use self::util::RwLock;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::sync::Arc;

#[test]
fn test_block_building_max_weight() {
	util::init_test_logger();
	global::set_mining_mode(global::ChainTypes::AutomatedTesting);

	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = ".grin_block_building_max_weight".to_string();
	clean_output_dir(db_root.clone());

	{
		let mut chain = ChainAdapter::init(db_root.clone()).unwrap();

		let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));

		// Convenient was to add a new block to the chain.
		let add_block =
			|prev_header: BlockHeader, txs: Vec<Transaction>, chain: &mut ChainAdapter| {
				let height = prev_header.height + 1;
				let key_id = ExtKeychain::derive_key_id(1, height as u32, 0, 0, 0);
				let fee = txs.iter().map(|x| x.fee()).sum();
				let reward = libtx::reward::output(&keychain, &key_id, fee, false).unwrap();
				let mut block = Block::new(&prev_header, txs, Difficulty::min(), reward).unwrap();

				// Set the prev_root to the prev hash for testing purposes (no MMR to obtain a root from).
				block.header.prev_root = prev_header.hash();

				chain.update_db_for_block(&block);
				block
			};

		// Initialize the chain/txhashset with an initial block
		// so we have a non-empty UTXO set.
		let block = add_block(BlockHeader::default(), vec![], &mut chain);
		let header = block.header;

		// Now create tx to spend that first coinbase (now matured).
		// Provides us with some useful outputs to test with.
		let initial_tx =
			test_transaction_spending_coinbase(&keychain, &header, vec![100, 200, 300]);

		// Mine that initial tx so we can spend it with multiple txs
		let block = add_block(header, vec![initial_tx], &mut chain);
		let header = block.header;

		// Initialize a new pool with our chain adapter.
		let pool = RwLock::new(test_setup(Arc::new(chain.clone()), verifier_cache));

		// Build some dependent txs to add to the txpool.
		// We will build a block from a subset of these.
		let txs = vec![
			test_transaction(&keychain, vec![100], vec![90, 1]),
			test_transaction(&keychain, vec![90], vec![80, 2]),
			test_transaction(&keychain, vec![200], vec![199]),
			test_transaction(&keychain, vec![300], vec![290, 3]),
			test_transaction(&keychain, vec![290], vec![280, 4]),
		];

		// Fees and weights of our original txs in insert order.
		assert_eq!(
			txs.iter().map(|x| x.fee()).collect::<Vec<_>>(),
			[9, 8, 1, 7, 6]
		);
		assert_eq!(
			txs.iter().map(|x| x.tx_weight()).collect::<Vec<_>>(),
			[8, 8, 4, 8, 8]
		);
		assert_eq!(
			txs.iter().map(|x| x.fee_to_weight()).collect::<Vec<_>>(),
			[1125, 1000, 250, 875, 750]
		);

		// Populate our txpool with the txs.
		{
			let mut write_pool = pool.write();
			for tx in txs {
				println!("***** {}", tx.fee_to_weight());
				write_pool
					.add_to_pool(test_source(), tx, false, &header)
					.unwrap();
			}
		}

		// Check we added them all to the txpool successfully.
		assert_eq!(pool.read().total_size(), 5);

		// Prepare some "mineable" txs from the txpool.
		// Note: We cannot fit all the txs from the txpool into a block.
		let txs = pool.read().prepare_mineable_transactions().unwrap();

		// Fees and weights of the "mineable" txs.
		assert_eq!(txs.iter().map(|x| x.fee()).collect::<Vec<_>>(), [9, 8, 7]);
		assert_eq!(
			txs.iter().map(|x| x.tx_weight()).collect::<Vec<_>>(),
			[8, 8, 8]
		);
		assert_eq!(
			txs.iter().map(|x| x.fee_to_weight()).collect::<Vec<_>>(),
			[1125, 1000, 875]
		);

		let block = add_block(header, txs, &mut chain);

		// Check contents of the block itself (including coinbase reward).
		assert_eq!(block.inputs().len(), 2);
		assert_eq!(block.outputs().len(), 6);
		assert_eq!(block.kernels().len(), 4);

		// Now reconcile the transaction pool with the new block
		// and check the resulting contents of the pool are what we expect.
		{
			let mut write_pool = pool.write();
			write_pool.reconcile_block(&block).unwrap();

			// We should still have 2 tx in the pool after accepting the new block.
			// This one exceeded the max block weight when building the block so
			// remained in the txpool.
			assert_eq!(write_pool.total_size(), 2);
		}
	}
	// Cleanup db directory
	clean_output_dir(db_root.clone());
}
