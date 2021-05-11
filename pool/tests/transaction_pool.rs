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

use self::core::core::{transaction, Weighting};
use self::core::global;
use self::keychain::{ExtKeychain, Keychain};
use self::pool::TxSource;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::sync::Arc;

/// Test we can add some txs to the pool (both stempool and txpool).
#[test]
fn test_the_transaction_pool() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_accept_fee_base(1);
	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.transaction_pool";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(Arc::new(ChainAdapter {
		chain: chain.clone(),
	}));

	// mine past HF4 to see effect of set_local_accept_fee_base
	add_some_blocks(&chain, 4 * 3, &keychain);
	let header = chain.head_header().unwrap();

	let header_1 = chain.get_header_by_height(1).unwrap();
	let initial_tx = test_transaction_spending_coinbase(
		&keychain,
		&header_1,
		vec![500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400],
	);

	// Add this tx to the pool (stem=false, direct to txpool).
	{
		pool.add_to_pool(test_source(), initial_tx, false, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 1);
	}

	// Test adding a tx that "double spends" an output currently spent by a tx
	// already in the txpool. In this case we attempt to spend the original coinbase twice.
	{
		let tx = test_transaction_spending_coinbase(&keychain, &header, vec![501]);
		assert!(pool.add_to_pool(test_source(), tx, false, &header).is_err());
	}

	// tx1 spends some outputs from the initial test tx.
	let tx1 = test_transaction(&keychain, vec![500, 600], vec![469, 569]);
	// tx2 spends some outputs from both tx1 and the initial test tx.
	let tx2 = test_transaction(&keychain, vec![469, 700], vec![498]);

	{
		// Check we have a single initial tx in the pool.
		assert_eq!(pool.total_size(), 1);

		// First, add a simple tx directly to the txpool (stem = false).
		pool.add_to_pool(test_source(), tx1.clone(), false, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 2);

		// Add another tx spending outputs from the previous tx.
		pool.add_to_pool(test_source(), tx2.clone(), false, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 3);
	}

	// Test adding the exact same tx multiple times (same kernel signature).
	// This will fail for stem=false during tx aggregation due to duplicate
	// outputs and duplicate kernels.
	{
		assert!(pool
			.add_to_pool(test_source(), tx1.clone(), false, &header)
			.is_err());
	}

	// Test adding a duplicate tx with the same input and outputs.
	// Note: not the *same* tx, just same underlying inputs/outputs.
	{
		let tx1a = test_transaction(&keychain, vec![500, 600], vec![469, 569]);
		assert!(pool
			.add_to_pool(test_source(), tx1a, false, &header)
			.is_err());
	}

	// Test adding a tx attempting to spend a non-existent output.
	{
		let bad_tx = test_transaction(&keychain, vec![10_001], vec![9_900]);
		assert!(pool
			.add_to_pool(test_source(), bad_tx, false, &header)
			.is_err());
	}

	// Test adding a tx that would result in a duplicate output (conflicts with
	// output from tx2). For reasons of security all outputs in the UTXO set must
	// be unique. Otherwise spending one will almost certainly cause the other
	// to be immediately stolen via a "replay" tx.
	{
		let tx = test_transaction(&keychain, vec![900], vec![498]);
		assert!(pool.add_to_pool(test_source(), tx, false, &header).is_err());
	}

	// Confirm the tx pool correctly identifies an invalid tx (already spent).
	{
		let tx3 = test_transaction(&keychain, vec![500], vec![467]);
		assert!(pool
			.add_to_pool(test_source(), tx3, false, &header)
			.is_err());
		assert_eq!(pool.total_size(), 3);
	}

	// Now add a couple of txs to the stempool (stem = true).
	{
		let tx = test_transaction(&keychain, vec![569], vec![538]);
		pool.add_to_pool(test_source(), tx, true, &header).unwrap();
		let tx2 = test_transaction(&keychain, vec![538], vec![507]);
		pool.add_to_pool(test_source(), tx2, true, &header).unwrap();
		assert_eq!(pool.total_size(), 3);
		assert_eq!(pool.stempool.size(), 2);
	}

	// Check we can take some entries from the stempool and "fluff" them into the
	// txpool. This also exercises multi-kernel txs.
	{
		let agg_tx = pool
			.stempool
			.all_transactions_aggregate(None)
			.unwrap()
			.unwrap();
		assert_eq!(agg_tx.kernels().len(), 2);
		pool.add_to_pool(test_source(), agg_tx, false, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 4);
		assert!(pool.stempool.is_empty());
	}

	// Adding a duplicate tx to the stempool will result in it being fluffed.
	// This handles the case of the stem path having a cycle in it.
	{
		let tx = test_transaction(&keychain, vec![507], vec![476]);
		pool.add_to_pool(test_source(), tx.clone(), true, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 4);
		assert_eq!(pool.txpool.size(), 4);
		assert_eq!(pool.stempool.size(), 1);

		// Duplicate stem tx so fluff, adding it to txpool and removing it from stempool.
		pool.add_to_pool(test_source(), tx.clone(), true, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 5);
		assert_eq!(pool.txpool.size(), 5);
		assert!(pool.stempool.is_empty());
	}

	// Now check we can correctly deaggregate a multi-kernel tx based on current
	// contents of the txpool.
	// We will do this be adding a new tx to the pool
	// that is a superset of a tx already in the pool.
	{
		let tx4 = test_transaction(&keychain, vec![800], vec![769]);

		// tx1 and tx2 are already in the txpool (in aggregated form)
		// tx4 is the "new" part of this aggregated tx that we care about
		let agg_tx = transaction::aggregate(&[tx1.clone(), tx2.clone(), tx4]).unwrap();

		agg_tx.validate(Weighting::AsTransaction).unwrap();

		pool.add_to_pool(test_source(), agg_tx, false, &header)
			.unwrap();
		assert_eq!(pool.total_size(), 6);
		let entry = pool.txpool.entries.last().unwrap();
		assert_eq!(entry.tx.kernels().len(), 1);
		assert_eq!(entry.src, TxSource::Deaggregate);
	}

	// Check we cannot "double spend" an output spent in a previous block.
	// We use the initial coinbase output here for convenience.
	{
		let double_spend_tx = test_transaction_spending_coinbase(&keychain, &header, vec![1000]);

		// check we cannot add a double spend to the stempool
		assert!(pool
			.add_to_pool(test_source(), double_spend_tx.clone(), true, &header)
			.is_err());

		// check we cannot add a double spend to the txpool
		assert!(pool
			.add_to_pool(test_source(), double_spend_tx.clone(), false, &header)
			.is_err());
	}

	// Cleanup db directory
	clean_output_dir(db_root.into());
}
