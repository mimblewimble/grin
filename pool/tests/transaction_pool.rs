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

extern crate chrono;
extern crate rand;

pub mod common;

use std::sync::Arc;
use util::RwLock;

use common::*;
use core::core::verifier_cache::LruVerifierCache;
use core::core::{transaction, Block, BlockHeader};
use core::libtx;
use core::pow::Difficulty;
use keychain::{ExtKeychain, Keychain};

/// Test we can add some txs to the pool (both stempool and txpool).
#[test]
fn test_the_transaction_pool() {
	let keychain: ExtKeychain = Keychain::from_random_seed().unwrap();

	let db_root = ".grin_transaction_pool".to_string();
	clean_output_dir(db_root.clone());
	let chain = Arc::new(ChainAdapter::init(db_root.clone()).unwrap());

	let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));

	// Initialize a new pool with our chain adapter.
	let pool = RwLock::new(test_setup(chain.clone(), verifier_cache.clone()));

	let header = {
		let height = 1;
		let key_id = ExtKeychain::derive_key_id(1, height as u32, 0, 0, 0);
		let reward = libtx::reward::output(&keychain, &key_id, 0, height).unwrap();
		let mut block =
			Block::new(&BlockHeader::default(), vec![], Difficulty::min(), reward).unwrap();

		chain.update_db_for_block(&block);

		block.header
	};

	// Now create tx to spend a coinbase, giving us some useful outputs for testing
	// with.
	let initial_tx = {
		test_transaction_spending_coinbase(
			&keychain,
			&header,
			vec![500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400],
		)
	};

	// Add this tx to the pool (stem=false, direct to txpool).
	{
		let mut write_pool = pool.write();
		write_pool
			.add_to_pool(test_source(), initial_tx, false, &header)
			.unwrap();
		assert_eq!(write_pool.total_size(), 1);
	}

	// Test adding a tx that "double spends" an output currently spent by a tx
	// already in the txpool. In this case we attempt to spend the original coinbase twice.
	{
		let tx = test_transaction_spending_coinbase(&keychain, &header, vec![501]);
		let mut write_pool = pool.write();
		assert!(
			write_pool
				.add_to_pool(test_source(), tx, true, &header)
				.is_err()
		);
	}

	// tx1 spends some outputs from the initial test tx.
	let tx1 = test_transaction(&keychain, vec![500, 600], vec![499, 599]);
	// tx2 spends some outputs from both tx1 and the initial test tx.
	let tx2 = test_transaction(&keychain, vec![499, 700], vec![498]);

	// Take a write lock and add a couple of tx entries to the pool.
	{
		let mut write_pool = pool.write();

		// Check we have a single initial tx in the pool.
		assert_eq!(write_pool.total_size(), 1);

		// First, add a simple tx to the pool in "stem" mode.
		write_pool
			.add_to_pool(test_source(), tx1.clone(), true, &header)
			.unwrap();
		assert_eq!(write_pool.total_size(), 1);
		assert_eq!(write_pool.stempool.size(), 1);

		// Add another tx spending outputs from the previous tx.
		write_pool
			.add_to_pool(test_source(), tx2.clone(), true, &header)
			.unwrap();
		assert_eq!(write_pool.total_size(), 1);
		assert_eq!(write_pool.stempool.size(), 2);
	}

	// Test adding the exact same tx multiple times (same kernel signature).
	// This will fail during tx aggregation due to duplicate outputs and duplicate
	// kernels.
	{
		let mut write_pool = pool.write();
		assert!(
			write_pool
				.add_to_pool(test_source(), tx1.clone(), true, &header)
				.is_err()
		);
	}

	// Test adding a duplicate tx with the same input and outputs.
	// Note: not the *same* tx, just same underlying inputs/outputs.
	{
		let tx1a = test_transaction(&keychain, vec![500, 600], vec![499, 599]);
		let mut write_pool = pool.write();
		assert!(
			write_pool
				.add_to_pool(test_source(), tx1a, true, &header)
				.is_err()
		);
	}

	// Test adding a tx attempting to spend a non-existent output.
	{
		let bad_tx = test_transaction(&keychain, vec![10_001], vec![10_000]);
		let mut write_pool = pool.write();
		assert!(
			write_pool
				.add_to_pool(test_source(), bad_tx, true, &header)
				.is_err()
		);
	}

	// Test adding a tx that would result in a duplicate output (conflicts with
	// output from tx2). For reasons of security all outputs in the UTXO set must
	// be unique. Otherwise spending one will almost certainly cause the other
	// to be immediately stolen via a "replay" tx.
	{
		let tx = test_transaction(&keychain, vec![900], vec![498]);
		let mut write_pool = pool.write();
		assert!(
			write_pool
				.add_to_pool(test_source(), tx, true, &header)
				.is_err()
		);
	}

	// Confirm the tx pool correctly identifies an invalid tx (already spent).
	{
		let mut write_pool = pool.write();
		let tx3 = test_transaction(&keychain, vec![500], vec![497]);
		assert!(
			write_pool
				.add_to_pool(test_source(), tx3, true, &header)
				.is_err()
		);
		assert_eq!(write_pool.total_size(), 1);
		assert_eq!(write_pool.stempool.size(), 2);
	}

	// Check we can take some entries from the stempool and "fluff" them into the
	// txpool. This also exercises multi-kernel txs.
	{
		let mut write_pool = pool.write();
		let agg_tx = write_pool
			.stempool
			.aggregate_transaction()
			.unwrap()
			.unwrap();
		assert_eq!(agg_tx.kernels().len(), 2);
		write_pool
			.add_to_pool(test_source(), agg_tx, false, &header)
			.unwrap();
		assert_eq!(write_pool.total_size(), 2);
	}

	// Now check we can correctly deaggregate a multi-kernel tx based on current
	// contents of the txpool.
	// We will do this be adding a new tx to the pool
	// that is a superset of a tx already in the pool.
	{
		let mut write_pool = pool.write();

		let tx4 = test_transaction(&keychain, vec![800], vec![799]);
		// tx1 and tx2 are already in the txpool (in aggregated form)
		// tx4 is the "new" part of this aggregated tx that we care about
		let agg_tx = transaction::aggregate(vec![tx1.clone(), tx2.clone(), tx4]).unwrap();

		agg_tx.validate(verifier_cache.clone()).unwrap();

		write_pool
			.add_to_pool(test_source(), agg_tx, false, &header)
			.unwrap();
		assert_eq!(write_pool.total_size(), 3);
		let entry = write_pool.txpool.entries.last().unwrap();
		assert_eq!(entry.tx.kernels().len(), 1);
		assert_eq!(entry.src.debug_name, "deagg");
	}

	// Check we cannot "double spend" an output spent in a previous block.
	// We use the initial coinbase output here for convenience.
	{
		let mut write_pool = pool.write();

		let double_spend_tx =
			{ test_transaction_spending_coinbase(&keychain, &header, vec![1000]) };

		// check we cannot add a double spend to the stempool
		assert!(
			write_pool
				.add_to_pool(test_source(), double_spend_tx.clone(), true, &header)
				.is_err()
		);

		// check we cannot add a double spend to the txpool
		assert!(
			write_pool
				.add_to_pool(test_source(), double_spend_tx.clone(), false, &header)
				.is_err()
		);
	}
}
