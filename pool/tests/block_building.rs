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

pub mod common;

use self::core::core::hash::Hashed;
use self::core::global;
use self::keychain::{ExtKeychain, Keychain};
use self::pool::PoolError;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::sync::Arc;

#[test]
fn test_transaction_pool_block_building() -> Result<(), PoolError> {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_accept_fee_base(1);
	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.block_building";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(Arc::new(ChainAdapter {
		chain: chain.clone(),
	}));

	// mine enough blocks to get past HF4
	add_some_blocks(&chain, 4 * 3, &keychain);

	let header_1 = chain.get_header_by_height(1).unwrap();

	// Now create tx to spend an early coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx =
		test_transaction_spending_coinbase(&keychain, &header_1, vec![100, 200, 300, 400]);

	// Mine that initial tx so we can spend it with multiple txs.
	add_block(&chain, &[initial_tx], &keychain);

	let header = chain.head_header().unwrap();

	let root_tx_1 = test_transaction(&keychain, vec![100, 200], vec![240]);
	let root_tx_2 = test_transaction(&keychain, vec![300], vec![270]);
	let root_tx_3 = test_transaction(&keychain, vec![400], vec![370]);

	let child_tx_1 = test_transaction(&keychain, vec![240], vec![210]);
	let child_tx_2 = test_transaction(&keychain, vec![370], vec![320]);

	{
		// Add the three root txs to the pool.
		pool.add_to_pool(test_source(), root_tx_1.clone(), false, &header)?;
		pool.add_to_pool(test_source(), root_tx_2.clone(), false, &header)?;
		pool.add_to_pool(test_source(), root_tx_3.clone(), false, &header)?;

		// Now add the two child txs to the pool.
		pool.add_to_pool(test_source(), child_tx_1.clone(), false, &header)?;
		pool.add_to_pool(test_source(), child_tx_2.clone(), false, &header)?;

		assert_eq!(pool.total_size(), 5);
	}

	let txs = pool.prepare_mineable_transactions()?;

	add_block(&chain, &txs, &keychain);

	// Get full block from head of the chain (block we just processed).
	let block = chain.get_block(&chain.head().unwrap().hash()).unwrap();

	// Check the block contains what we expect.
	assert_eq!(block.inputs().len(), 4);
	assert_eq!(block.outputs().len(), 4);
	assert_eq!(block.kernels().len(), 6);

	assert!(block.kernels().contains(&root_tx_1.kernels()[0]));
	assert!(block.kernels().contains(&root_tx_2.kernels()[0]));
	assert!(block.kernels().contains(&root_tx_3.kernels()[0]));
	assert!(block.kernels().contains(&child_tx_1.kernels()[0]));
	assert!(block.kernels().contains(&child_tx_1.kernels()[0]));

	// Now reconcile the transaction pool with the new block
	// and check the resulting contents of the pool are what we expect.
	{
		pool.reconcile_block(&block)?;
		assert_eq!(pool.total_size(), 0);
	}

	// Cleanup db directory
	clean_output_dir(db_root.into());

	Ok(())
}
