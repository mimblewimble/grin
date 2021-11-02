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

//! Test coverage for block building at the limit of max_block_weight.

pub mod common;
use self::core::core::hash::Hashed;
use self::core::global;
use self::keychain::{ExtKeychain, Keychain};
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::sync::Arc;

#[test]
fn test_block_building_max_weight() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_accept_fee_base(1);

	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.block_max_weight";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(Arc::new(ChainAdapter {
		chain: chain.clone(),
	}));

	// mine past HF4 to see effect of set_local_accept_fee_base
	add_some_blocks(&chain, 4 * 3, &keychain);

	let header_1 = chain.get_header_by_height(1).unwrap();

	// Now create tx to spend an early coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx = test_transaction_spending_coinbase(
		&keychain,
		&header_1,
		vec![1_000_000, 2_000_000, 3_000_000, 10_000_000],
	);

	// Mine that initial tx so we can spend it with multiple txs.
	add_block(&chain, &[initial_tx], &keychain);

	let header = chain.head_header().unwrap();

	// Build some dependent txs to add to the txpool.
	// We will build a block from a subset of these.
	let txs = vec![
		test_transaction(
			&keychain,
			vec![10_000_000],
			vec![3_900_000, 1_300_000, 1_200_000, 1_100_000],
		),
		test_transaction(&keychain, vec![1_000_000], vec![900_000, 10_000]),
		test_transaction(&keychain, vec![900_000], vec![800_000, 20_000]),
		test_transaction(&keychain, vec![2_000_000], vec![1_970_000]),
		test_transaction(&keychain, vec![3_000_000], vec![2_900_000, 30_000]),
		test_transaction(&keychain, vec![2_900_000], vec![2_800_000, 40_000]),
	];

	// Fees and weights of our original txs in insert order.
	assert_eq!(
		txs.iter().map(|x| x.fee()).collect::<Vec<_>>(),
		[2_500_000, 90_000, 80_000, 30_000, 70_000, 60_000]
	);
	assert_eq!(
		txs.iter().map(|x| x.weight()).collect::<Vec<_>>(),
		[88, 46, 46, 25, 46, 46]
	);
	assert_eq!(
		txs.iter().map(|x| x.fee_rate()).collect::<Vec<_>>(),
		[28409, 1956, 1739, 1200, 1521, 1304]
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
		[2_500_000, 90_000, 80_000, 70_000]
	);
	assert_eq!(
		txs.iter().map(|x| x.weight()).collect::<Vec<_>>(),
		[88, 46, 46, 46]
	);
	assert_eq!(
		txs.iter().map(|x| x.fee_rate()).collect::<Vec<_>>(),
		[28409, 1956, 1739, 1521]
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
