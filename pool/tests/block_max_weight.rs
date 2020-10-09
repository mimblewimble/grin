// Copyright 2020 The Grin Developers
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
use self::core::global;
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
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);

	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.block_max_weight";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));
	let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(
		Arc::new(ChainAdapter {
			chain: chain.clone(),
		}),
		verifier_cache,
	);

	add_some_blocks(&chain, 3, &keychain);

	let header_1 = chain.get_header_by_height(1).unwrap();

	// Now create tx to spend an early coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx =
		test_transaction_spending_coinbase(&keychain, &header_1, vec![100, 200, 300, 1000]);

	// Mine that initial tx so we can spend it with multiple txs.
	add_block(&chain, &[initial_tx], &keychain);

	let header = chain.head_header().unwrap();

	// Build some dependent txs to add to the txpool.
	// We will build a block from a subset of these.
	let txs = vec![
		test_transaction(&keychain, vec![1000], vec![390, 130, 120, 110]),
		test_transaction(&keychain, vec![100], vec![90, 1]),
		test_transaction(&keychain, vec![90], vec![80, 2]),
		test_transaction(&keychain, vec![200], vec![199]),
		test_transaction(&keychain, vec![300], vec![290, 3]),
		test_transaction(&keychain, vec![290], vec![280, 4]),
	];

	// Fees and weights of our original txs in insert order.
	assert_eq!(
		txs.iter().map(|x| x.fee()).collect::<Vec<_>>(),
		[250, 9, 8, 1, 7, 6]
	);
	assert_eq!(
		txs.iter().map(|x| x.tx_weight()).collect::<Vec<_>>(),
		[16, 8, 8, 4, 8, 8]
	);
	assert_eq!(
		txs.iter().map(|x| x.fee_to_weight()).collect::<Vec<_>>(),
		[15625, 1125, 1000, 250, 875, 750]
	);

	// Populate our txpool with the txs.
	for tx in txs {
		pool.add_to_pool(test_source(), tx, false, &header).unwrap();
	}

	// Check we added them all to the txpool successfully.
	assert_eq!(pool.total_size(), 6);

	// // Prepare some "mineable" txs from the txpool.
	// // Note: We cannot fit all the txs from the txpool into a block.
	let txs = pool.prepare_mineable_transactions().unwrap();

	// Fees and weights of the "mineable" txs.
	assert_eq!(
		txs.iter().map(|x| x.fee()).collect::<Vec<_>>(),
		[250, 9, 8, 7]
	);
	assert_eq!(
		txs.iter().map(|x| x.tx_weight()).collect::<Vec<_>>(),
		[16, 8, 8, 8]
	);
	assert_eq!(
		txs.iter().map(|x| x.fee_to_weight()).collect::<Vec<_>>(),
		[15625, 1125, 1000, 875]
	);

	add_block(&chain, &txs, &keychain);
	let block = chain.get_block(&chain.head().unwrap().hash()).unwrap();

	// Check contents of the block itself (including coinbase reward).
	assert_eq!(block.inputs().len(), 3);
	assert_eq!(block.outputs().len(), 10);
	assert_eq!(block.kernels().len(), 5);

	// Now reconcile the transaction pool with the new block
	// and check the resulting contents of the pool are what we expect.
	pool.reconcile_block(&block).unwrap();

	// We should still have 2 tx in the pool after accepting the new block.
	// This one exceeded the max block weight when building the block so
	// remained in the txpool.
	assert_eq!(pool.total_size(), 2);

	// Cleanup db directory
	clean_output_dir(db_root.into());
}
