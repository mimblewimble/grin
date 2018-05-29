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

//! Top-level Pool tests

extern crate blake2_rfc as blake2;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_pool as pool;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate rand;
extern crate time;

use std::collections::HashMap;

use core::core::block;
use core::core::transaction::{self, ProofMessageElements};
use core::core::{OutputIdentifier, Transaction};

use blockchain::{DummyChain, DummyChainImpl, DummyOutputSet};
use core::core::Proof;
use core::core::hash::{Hash, Hashed};
use core::core::pmmr::MerkleProof;
use core::core::target::Difficulty;
use core::global;
use core::global::ChainTypes;
use pool::*;
use std::sync::{Arc, RwLock};
use types::PoolError::InvalidTx;

use keychain::Keychain;
use wallet::libtx::{build, proof, reward};

use pool::types::*;

macro_rules! expect_output_parent {
	($pool:expr, $expected:pat, $( $output:expr ),+ ) => {
		$(
			match $pool
			.search_for_best_output(
				&OutputIdentifier::from_output(&test_output($output))
			) {
				$expected => {},
				x => panic!(
					"Unexpected result from output search for {:?}, got {:?}",
					$output,
					x,
				),
			};
		)*
	}
}

#[test]
/// A basic test; add a pair of transactions to the pool.
fn test_basic_pool_add() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
	// We want this transaction to be rooted in the blockchain.
	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(6))
		.with_output(test_output(7))
		.with_output(test_output(8));

	// Prepare a second transaction, connected to the first.
	let child_transaction = test_transaction(vec![11, 3], vec![12]);

	dummy_chain.update_output_set(new_output);

	// To mirror how this construction is intended to be used, the pool
	// is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the transaction rooted in the blockchain
		let result = write_pool.add_to_memory_pool(test_source(), parent_transaction, false);
		if result.is_err() {
			panic!("got an error adding parent tx: {:?}", result.err().unwrap());
		}

		// Now, add the transaction connected as a child to the first
		let child_result = write_pool.add_to_memory_pool(test_source(), child_transaction, false);

		if child_result.is_err() {
			panic!(
				"got an error adding child tx: {:?}",
				child_result.err().unwrap()
			);
		}
	}

	// Now take the read lock and use a few exposed methods to check consistency
	{
		let read_pool = pool.read().unwrap();
		assert_eq!(read_pool.total_size(), 2);
		expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
		expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
		expect_output_parent!(read_pool, Parent::Unknown, 20);
	}
}

#[test]
/// Attempt to add a multi kernel transaction to the mempool
fn test_multikernel_pool_add() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
	// We want this transaction to be rooted in the blockchain.
	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(6))
		.with_output(test_output(7))
		.with_output(test_output(8));

	// Prepare a second transaction, connected to the first.
	let child_transaction = test_transaction(vec![11, 3], vec![12]);

	let txs = vec![parent_transaction, child_transaction];
	let multi_kernel_transaction = transaction::aggregate_with_cut_through(txs).unwrap();

	dummy_chain.update_output_set(new_output);

	// To mirror how this construction is intended to be used, the pool
	// is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the transaction rooted in the blockchain
		let result = write_pool.add_to_memory_pool(test_source(), multi_kernel_transaction, false);
		if result.is_err() {
			panic!(
				"got an error adding multi-kernel tx: {:?}",
				result.err().unwrap()
			);
		}
	}

	// Now take the read lock and use a few exposed methods to check consistency
	{
		let read_pool = pool.read().unwrap();
		assert_eq!(read_pool.total_size(), 1);
		expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 5);
		expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
		expect_output_parent!(read_pool, Parent::Unknown, 11, 3, 20);
	}
}

#[test]
/// Attempt to deaggregate a multi_kernel transaction
/// Push the parent transaction in the mempool then send a multikernel tx
/// containing it and a child transaction In the end, the pool should contain
/// both transactions.
fn test_multikernel_deaggregate() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let transaction1 = test_transaction_with_offset(vec![5], vec![1]);
	println!("{:?}", transaction1.validate());
	let transaction2 = test_transaction_with_offset(vec![8], vec![2]);

	// We want these transactions to be rooted in the blockchain.
	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(8));

	dummy_chain.update_output_set(new_output);

	// To mirror how this construction is intended to be used, the pool
	// is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the first transaction
		let result = write_pool.add_to_memory_pool(test_source(), transaction1.clone(), false);
		if result.is_err() {
			panic!("got an error adding tx 1: {:?}", result.err().unwrap());
		}
	}

	let txs = vec![transaction1.clone(), transaction2.clone()];
	let multi_kernel_transaction = transaction::aggregate(txs).unwrap();

	let found_tx: Transaction;
	// Now take the read lock and attempt to deaggregate the transaction
	{
		let read_pool = pool.read().unwrap();
		found_tx = read_pool
			.deaggregate_transaction(multi_kernel_transaction)
			.unwrap();

		// Test the retrived transactions
		assert_eq!(transaction2, found_tx);
	}

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 1);

		// First, add the transaction rooted in the blockchain
		let result = write_pool.add_to_memory_pool(test_source(), found_tx.clone(), false);
		if result.is_err() {
			panic!("got an error adding child tx: {:?}", result.err().unwrap());
		}
	}

	// Now take the read lock and use a few exposed methods to check consistency
	{
		let read_pool = pool.read().unwrap();
		assert_eq!(read_pool.total_size(), 2);
		expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 1, 2);
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 5, 8);
		expect_output_parent!(read_pool, Parent::Unknown, 11, 3, 20);
	}
}

#[test]
/// Attempt to add a bad multi kernel transaction to the mempool should get
/// rejected
fn test_bad_multikernel_pool_add() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
	// We want this transaction to be rooted in the blockchain.
	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(6))
		.with_output(test_output(7))
		.with_output(test_output(8));

	// Prepare a second transaction, connected to the first.
	let child_transaction1 = test_transaction(vec![11, 3], vec![12]);
	let child_transaction2 = test_transaction(vec![11, 3], vec![10]);

	let txs = vec![parent_transaction, child_transaction1, child_transaction2];
	let bad_multi_kernel_transaction = transaction::aggregate(txs).unwrap();

	dummy_chain.update_output_set(new_output);

	// To mirror how this construction is intended to be used, the pool
	// is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the transaction rooted in the blockchain
		let result =
			write_pool.add_to_memory_pool(test_source(), bad_multi_kernel_transaction, false);
		assert!(result.is_err());
	}
}

#[test]
/// A basic test; add a transaction to the pool and add the child to the
/// stempool
fn test_pool_stempool_add() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
	// We want this transaction to be rooted in the blockchain.
	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(6))
		.with_output(test_output(7))
		.with_output(test_output(8));

	// Prepare a second transaction, connected to the first.
	let child_transaction = test_transaction(vec![11, 3], vec![12]);

	dummy_chain.update_output_set(new_output);

	// To mirror how this construction is intended to be used, the pool
	// is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the transaction rooted in the blockchain
		let result = write_pool.add_to_memory_pool(test_source(), parent_transaction, false);
		if result.is_err() {
			panic!("got an error adding parent tx: {:?}", result.err().unwrap());
		}

		// Now, add the transaction connected as a child to the first
		let child_result = write_pool.add_to_memory_pool(test_source(), child_transaction, true);

		if child_result.is_err() {
			panic!(
				"got an error adding child tx: {:?}",
				child_result.err().unwrap()
			);
		}
	}

	// Now take the read lock and use a few exposed methods to check consistency
	{
		let read_pool = pool.read().unwrap();
		assert_eq!(read_pool.total_size(), 2);
		if read_pool.stempool.num_transactions() == 0 {
			expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
		} else {
			expect_output_parent!(read_pool, Parent::StemPoolTransaction{tx_ref: _}, 12);
		}
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
		expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
		expect_output_parent!(read_pool, Parent::Unknown, 20);
	}
}

#[test]
/// A basic test; add a transaction to the stempool and one the regular
/// transaction pool Child transaction should be added to the stempool.
fn test_stempool_pool_add() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
	// We want this transaction to be rooted in the blockchain.
	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(6))
		.with_output(test_output(7))
		.with_output(test_output(8));

	// Prepare a second transaction, connected to the first.
	let child_transaction = test_transaction(vec![11, 3], vec![12]);

	dummy_chain.update_output_set(new_output);

	// To mirror how this construction is intended to be used, the pool
	// is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the transaction rooted in the blockchain
		let result = write_pool.add_to_memory_pool(test_source(), parent_transaction, true);
		if result.is_err() {
			panic!("got an error adding parent tx: {:?}", result.err().unwrap());
		}

		// Now, add the transaction connected as a child to the first
		let child_result = write_pool.add_to_memory_pool(test_source(), child_transaction, false);
		if child_result.is_err() {
			panic!(
				"got an error adding child tx: {:?}",
				child_result.err().unwrap()
			);
		}
	}

	// Now take the read lock and use a few exposed methods to check consistency
	{
		let read_pool = pool.read().unwrap();
		// First transaction is a stem transaction. In that case the child transaction
		// should be force stem
		assert_eq!(read_pool.total_size(), 2);
		// Parent has been directly fluffed
		if read_pool.stempool.num_transactions() == 0 {
			expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
		} else {
			expect_output_parent!(read_pool, Parent::StemPoolTransaction{tx_ref: _}, 12);
		}
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
		expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
		expect_output_parent!(read_pool, Parent::Unknown, 20);
	}
}

#[test]
/// Testing various expected error conditions
pub fn test_pool_add_error() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let new_output = DummyOutputSet::empty()
		.with_output(test_output(5))
		.with_output(test_output(6))
		.with_output(test_output(7));

	dummy_chain.update_output_set(new_output);

	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First expected failure: duplicate output
		let duplicate_tx = test_transaction(vec![5, 6], vec![7]);

		match write_pool.add_to_memory_pool(test_source(), duplicate_tx, false) {
			Ok(_) => panic!("Got OK from add_to_memory_pool when dup was expected"),
			Err(x) => {
				match x {
					PoolError::DuplicateOutput {
						other_tx,
						in_chain,
						output,
					} => if other_tx.is_some() || !in_chain || output != test_output(7).commitment()
					{
						panic!("Unexpected parameter in DuplicateOutput: {:?}", x);
					},
					_ => panic!(
						"Unexpected error when adding duplicate output transaction: {:?}",
						x
					),
				};
			}
		};

		// To test DoubleSpend and AlreadyInPool conditions, we need to add
		// a valid transaction.
		let valid_transaction = test_transaction(vec![5, 6], vec![9]);

		match write_pool.add_to_memory_pool(test_source(), valid_transaction.clone(), false) {
			Ok(_) => {}
			Err(x) => panic!("Unexpected error while adding a valid transaction: {:?}", x),
		};

		// Now, test a DoubleSpend by consuming the same blockchain unspent
		// as valid_transaction:
		let double_spend_transaction = test_transaction(vec![6], vec![2]);

		match write_pool.add_to_memory_pool(test_source(), double_spend_transaction, false) {
			Ok(_) => panic!("Expected error when adding double spend, got Ok"),
			Err(x) => {
				match x {
					PoolError::DoubleSpend {
						other_tx: _,
						spent_output,
					} => if spent_output != test_output(6).commitment() {
						panic!("Unexpected parameter in DoubleSpend: {:?}", x);
					},
					_ => panic!(
						"Unexpected error when adding double spend transaction: {:?}",
						x
					),
				};
			}
		};

		// Note, this used to work as expected, but after aggsig implementation
		// creating another transaction with the same inputs/outputs doesn't create
		// the same hash ID due to the random nonces in an aggsig. This
		// will instead throw a (correct as well) Already spent error. An AlreadyInPool
		// error can only come up in the case of the exact same transaction being
		// added
		//let already_in_pool = test_transaction(vec![5, 6], vec![9]);

		match write_pool.add_to_memory_pool(test_source(), valid_transaction, false) {
			Ok(_) => panic!("Expected error when adding already in pool, got Ok"),
			Err(x) => {
				match x {
					PoolError::AlreadyInPool => {}
					_ => panic!("Unexpected error when adding already in pool tx: {:?}", x),
				};
			}
		};

		assert_eq!(write_pool.total_size(), 1);

		// now attempt to add a timelocked tx to the pool
		// should fail as invalid based on current height
		let timelocked_tx_1 = timelocked_transaction(vec![9], vec![5], 10);
		match write_pool.add_to_memory_pool(test_source(), timelocked_tx_1, false) {
			Err(PoolError::ImmatureTransaction {
				lock_height: height,
			}) => {
				assert_eq!(height, 10);
			}
			Err(e) => panic!("expected ImmatureTransaction error here - {:?}", e),
			Ok(_) => panic!("expected ImmatureTransaction error here"),
		};
	}
}

#[test]
fn test_immature_coinbase() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let mut dummy_chain = DummyChainImpl::new();
	let proof_size = global::proofsize();

	let lock_height = 1 + global::coinbase_maturity();
	assert_eq!(lock_height, 4);

	let coinbase_output = test_coinbase_output(15);
	dummy_chain.update_output_set(DummyOutputSet::empty().with_output(coinbase_output));

	let chain_ref = Arc::new(dummy_chain);
	let pool = RwLock::new(test_setup(&chain_ref));

	{
		let mut write_pool = pool.write().unwrap();

		let coinbase_header = block::BlockHeader {
			height: 1,
			pow: Proof::random(proof_size),
			..block::BlockHeader::default()
		};
		chain_ref.store_head_header(&coinbase_header);

		let head_header = block::BlockHeader {
			height: 2,
			pow: Proof::random(proof_size),
			..block::BlockHeader::default()
		};
		chain_ref.store_head_header(&head_header);

		let txn = test_transaction_with_coinbase_input(15, coinbase_header.hash(), vec![10, 3]);
		let result = write_pool.add_to_memory_pool(test_source(), txn, false);
		match result {
			Err(InvalidTx(transaction::Error::ImmatureCoinbase)) => {}
			_ => panic!("expected ImmatureCoinbase error here"),
		};

		let head_header = block::BlockHeader {
			height: 4,
			..block::BlockHeader::default()
		};
		chain_ref.store_head_header(&head_header);

		let txn = test_transaction_with_coinbase_input(15, coinbase_header.hash(), vec![10, 3]);
		let result = write_pool.add_to_memory_pool(test_source(), txn, false);
		match result {
			Ok(_) => {}
			Err(_) => panic!("this should not return an error here"),
		};
	}
}

#[test]
/// Testing an expected orphan
fn test_add_orphan() {
	// TODO we need a test here
}

#[test]
fn test_zero_confirmation_reconciliation() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	// single Output
	let new_output = DummyOutputSet::empty().with_output(test_output(100));

	dummy_chain.update_output_set(new_output);
	let chain_ref = Arc::new(dummy_chain);
	let pool = RwLock::new(test_setup(&chain_ref));

	// now create two txs
	// tx1 spends the Output
	// tx2 spends output from tx1
	let tx1 = test_transaction(vec![100], vec![90]);
	let tx2 = test_transaction(vec![90], vec![80]);

	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// now add both txs to the pool (tx2 spends tx1 with zero confirmations)
		// both should be accepted if tx1 added before tx2
		write_pool
			.add_to_memory_pool(test_source(), tx1, false)
			.unwrap();
		write_pool
			.add_to_memory_pool(test_source(), tx2, false)
			.unwrap();

		assert_eq!(write_pool.pool_size(), 2);
	}

	let txs: Vec<transaction::Transaction>;
	{
		let read_pool = pool.read().unwrap();
		let mut mineable_txs = read_pool.prepare_mineable_transactions(3);
		txs = mineable_txs.drain(..).collect();

		// confirm we can preparing both txs for mining here
		// one root tx in the pool, and one non-root vertex in the pool
		assert_eq!(txs.len(), 2);
	}

	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward = reward::output(&keychain, &key_id, fees, 0).unwrap();

	// now "mine" the block passing in the mineable txs from earlier
	let block = block::Block::new(
		&block::BlockHeader::default(),
		txs.iter().collect(),
		Difficulty::one(),
		reward,
	).unwrap();

	// now apply the block to ensure the chainstate is updated before we reconcile
	chain_ref.apply_block(&block);

	// now reconcile the block
	// we should evict both txs here
	{
		let mut write_pool = pool.write().unwrap();
		let evicted_transactions = write_pool.reconcile_block(&block).unwrap();
		assert_eq!(evicted_transactions.len(), 2);
	}

	// check the pool is consistent after reconciling the block
	// we should have zero txs in the pool (neither roots nor non-roots)
	{
		let read_pool = pool.write().unwrap();
		assert_eq!(read_pool.pool.len_vertices(), 0);
		assert_eq!(read_pool.pool.len_roots(), 0);
	}
}

#[test]
/// Testing block reconciliation
fn test_block_reconciliation() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let new_output = DummyOutputSet::empty()
		.with_output(test_output(10))
		.with_output(test_output(20))
		.with_output(test_output(30))
		.with_output(test_output(40));

	dummy_chain.update_output_set(new_output);

	let chain_ref = Arc::new(dummy_chain);

	let pool = RwLock::new(test_setup(&chain_ref));

	// Preparation: We will introduce a three root pool transactions.
	// 1. A transaction that should be invalidated because it is exactly
	//  contained in the block.
	// 2. A transaction that should be invalidated because the input is
	//  consumed in the block, although it is not exactly consumed.
	// 3. A transaction that should remain after block reconciliation.
	let block_transaction = test_transaction(vec![10], vec![8]);
	let conflict_transaction = test_transaction(vec![20], vec![12, 6]);
	let valid_transaction = test_transaction(vec![30], vec![13, 15]);

	// We will also introduce a few children:
	// 4. A transaction that descends from transaction 1, that is in
	//  turn exactly contained in the block.
	let block_child = test_transaction(vec![8], vec![5, 1]);
	// 5. A transaction that descends from transaction 4, that is not
	//  contained in the block at all and should be valid after
	//  reconciliation.
	let pool_child = test_transaction(vec![5], vec![3]);
	// 6. A transaction that descends from transaction 2 that does not
	//  conflict with anything in the block in any way, but should be
	//  invalidated (orphaned).
	let conflict_child = test_transaction(vec![12], vec![2]);
	// 7. A transaction that descends from transaction 2 that should be
	//  valid due to its inputs being satisfied by the block.
	let conflict_valid_child = test_transaction(vec![6], vec![4]);
	// 8. A transaction that descends from transaction 3 that should be
	//  invalidated due to an output conflict.
	let valid_child_conflict = test_transaction(vec![13], vec![9]);
	// 9. A transaction that descends from transaction 3 that should remain
	//  valid after reconciliation.
	let valid_child_valid = test_transaction(vec![15], vec![11]);
	// 10. A transaction that descends from both transaction 6 and
	//  transaction 9
	let mixed_child = test_transaction(vec![2, 11], vec![7]);

	// Add transactions.
	// Note: There are some ordering constraints that must be followed here
	// until orphans is 100% implemented. Once the orphans process has
	// stabilized, we can mix these up to exercise that path a bit.
	let mut txs_to_add = vec![
		block_transaction,
		conflict_transaction,
		valid_transaction,
		block_child,
		pool_child,
		conflict_child,
		conflict_valid_child,
		valid_child_conflict,
		valid_child_valid,
		mixed_child,
	];

	let expected_pool_size = txs_to_add.len();

	// First we add the above transactions to the pool; all should be
	// accepted.
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		for tx in txs_to_add.drain(..) {
			write_pool
				.add_to_memory_pool(test_source(), tx, false)
				.unwrap();
		}

		assert_eq!(write_pool.total_size(), expected_pool_size);
	}
	// Now we prepare the block that will cause the above condition.
	// First, the transactions we want in the block:
	// - Copy of 1
	let block_tx_1 = test_transaction(vec![10], vec![8]);
	// - Conflict w/ 2, satisfies 7
	let block_tx_2 = test_transaction(vec![20], vec![6]);
	// - Copy of 4
	let block_tx_3 = test_transaction(vec![8], vec![5, 1]);
	// - Output conflict w/ 8
	let block_tx_4 = test_transaction(vec![40], vec![9, 1]);
	let block_transactions = vec![&block_tx_1, &block_tx_2, &block_tx_3, &block_tx_4];

	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let fees = block_transactions.iter().map(|tx| tx.fee()).sum();
	let reward = reward::output(&keychain, &key_id, fees, 0).unwrap();

	let block = block::Block::new(
		&block::BlockHeader::default(),
		block_transactions,
		Difficulty::one(),
		reward,
	).unwrap();

	chain_ref.apply_block(&block);

	// Block reconciliation
	{
		let mut write_pool = pool.write().unwrap();

		let evicted_transactions = write_pool.reconcile_block(&block);

		assert!(evicted_transactions.is_ok());

		assert_eq!(evicted_transactions.unwrap().len(), 6);

		// TODO: Txids are not yet deterministic. When they are, we should
		// check the specific transactions that were evicted.
	}

	// Using the pool's methods to validate a few end conditions.
	{
		let read_pool = pool.read().unwrap();

		assert_eq!(read_pool.total_size(), 4);

		// We should have available blockchain outputs
		expect_output_parent!(read_pool, Parent::BlockTransaction, 9, 1);

		// We should have spent blockchain outputs
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 5, 6);

		// We should have spent pool references
		expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 15);

		// We should have unspent pool references
		expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 3, 11, 13);

		// References internal to the block should be unknown
		expect_output_parent!(read_pool, Parent::Unknown, 8);

		// Evicted transactions should have unknown outputs
		expect_output_parent!(read_pool, Parent::Unknown, 2, 7);
	}
}

#[test]
/// Test transaction selection and block building.
fn test_block_building() {
	// Add a handful of transactions
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = block::BlockHeader {
		height: 1,
		..block::BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	let new_output = DummyOutputSet::empty()
		.with_output(test_output(10))
		.with_output(test_output(20))
		.with_output(test_output(30))
		.with_output(test_output(40));

	dummy_chain.update_output_set(new_output);

	let chain_ref = Arc::new(dummy_chain);

	let pool = RwLock::new(test_setup(&chain_ref));

	let root_tx_1 = test_transaction(vec![10, 20], vec![24]);
	let root_tx_2 = test_transaction(vec![30], vec![28]);
	let root_tx_3 = test_transaction(vec![40], vec![38]);

	let child_tx_1 = test_transaction(vec![24], vec![22]);
	let child_tx_2 = test_transaction(vec![38], vec![32]);

	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		assert!(
			write_pool
				.add_to_memory_pool(test_source(), root_tx_1, false)
				.is_ok()
		);
		assert!(
			write_pool
				.add_to_memory_pool(test_source(), root_tx_2, false)
				.is_ok()
		);
		assert!(
			write_pool
				.add_to_memory_pool(test_source(), root_tx_3, false)
				.is_ok()
		);
		assert!(
			write_pool
				.add_to_memory_pool(test_source(), child_tx_1, false)
				.is_ok()
		);
		assert!(
			write_pool
				.add_to_memory_pool(test_source(), child_tx_2, false)
				.is_ok()
		);

		assert_eq!(write_pool.total_size(), 5);
	}

	// Request blocks
	let block: block::Block;
	let mut txs: Vec<transaction::Transaction>;
	{
		let read_pool = pool.read().unwrap();
		txs = read_pool.prepare_mineable_transactions(3);
		assert_eq!(txs.len(), 3);
		// TODO: This is ugly, either make block::new take owned
		// txs instead of mut refs, or change
		// prepare_mineable_transactions to return mut refs
		let block_txs: Vec<transaction::Transaction> = txs.drain(..).collect();
		let tx_refs: Vec<&transaction::Transaction> = block_txs.iter().collect();

		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let fees = tx_refs.iter().map(|tx| tx.fee()).sum();
		let reward = reward::output(&keychain, &key_id, fees, 0).unwrap();
		block = block::Block::new(
			&block::BlockHeader::default(),
			tx_refs,
			Difficulty::one(),
			reward,
		).unwrap();
	}

	chain_ref.apply_block(&block);
	// Reconcile block
	{
		let mut write_pool = pool.write().unwrap();

		let evicted_transactions = write_pool.reconcile_block(&block);

		assert!(evicted_transactions.is_ok());

		assert_eq!(evicted_transactions.unwrap().len(), 3);
		assert_eq!(write_pool.total_size(), 2);
	}
}

fn test_setup(dummy_chain: &Arc<DummyChainImpl>) -> TransactionPool<DummyChainImpl> {
	TransactionPool {
		config: PoolConfig {
			accept_fee_base: 0,
			max_pool_size: 10_000,
			dandelion_probability: 90,
			dandelion_embargo: 30,
		},
		time_stem_transactions: HashMap::new(),
		stem_transactions: HashMap::new(),
		transactions: HashMap::new(),
		stempool: Pool::empty(),
		pool: Pool::empty(),
		orphans: Orphans::empty(),
		blockchain: dummy_chain.clone(),
		adapter: Arc::new(NoopAdapter {}),
	}
}

/// Cobble together a test transaction for testing the transaction pool.
///
/// Connectivity here is the most important element.
/// Every output is given a blinding key equal to its value, so that the
/// entire commitment can be derived deterministically from just the value.
///
/// Fees are the remainder between input and output values,
/// so the numbers should make sense.
fn test_transaction(input_values: Vec<u64>, output_values: Vec<u64>) -> transaction::Transaction {
	let keychain = keychain_for_tests();

	let input_sum = input_values.iter().sum::<u64>() as i64;
	let output_sum = output_values.iter().sum::<u64>() as i64;

	let fees: i64 = input_sum - output_sum;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	for input_value in input_values {
		let key_id = keychain.derive_key_id(input_value as u32).unwrap();
		tx_elements.push(build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(build::output(output_value, key_id));
	}
	tx_elements.push(build::with_fee(fees as u64));

	build::transaction(tx_elements, &keychain).unwrap()
}

fn test_transaction_with_offset(
	input_values: Vec<u64>,
	output_values: Vec<u64>,
) -> transaction::Transaction {
	let keychain = keychain_for_tests();

	let input_sum = input_values.iter().sum::<u64>() as i64;
	let output_sum = output_values.iter().sum::<u64>() as i64;

	let fees: i64 = input_sum - output_sum;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	for input_value in input_values {
		let key_id = keychain.derive_key_id(input_value as u32).unwrap();
		tx_elements.push(build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(build::output(output_value, key_id));
	}
	tx_elements.push(build::with_fee(fees as u64));

	build::transaction_with_offset(tx_elements, &keychain).unwrap()
}

fn test_transaction_with_coinbase_input(
	input_value: u64,
	input_block_hash: Hash,
	output_values: Vec<u64>,
) -> transaction::Transaction {
	let keychain = keychain_for_tests();

	let output_sum = output_values.iter().sum::<u64>() as i64;

	let fees: i64 = input_value as i64 - output_sum;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	let merkle_proof = MerkleProof {
		node: Hash::default(),
		root: Hash::default(),
		peaks: vec![Hash::default()],
		..MerkleProof::default()
	};

	let key_id = keychain.derive_key_id(input_value as u32).unwrap();
	tx_elements.push(build::coinbase_input(
		input_value,
		input_block_hash,
		merkle_proof,
		key_id,
	));

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(build::output(output_value, key_id));
	}
	tx_elements.push(build::with_fee(fees as u64));

	build::transaction(tx_elements, &keychain).unwrap()
}

/// Very un-dry way of building a vanilla tx and adding a lock_height to it.
/// TODO - rethink this.
fn timelocked_transaction(
	input_values: Vec<u64>,
	output_values: Vec<u64>,
	lock_height: u64,
) -> transaction::Transaction {
	let keychain = keychain_for_tests();

	let fees: i64 =
		input_values.iter().sum::<u64>() as i64 - output_values.iter().sum::<u64>() as i64;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	for input_value in input_values {
		let key_id = keychain.derive_key_id(input_value as u32).unwrap();
		tx_elements.push(build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(build::output(output_value, key_id));
	}
	tx_elements.push(build::with_fee(fees as u64));

	tx_elements.push(build::with_lock_height(lock_height));
	build::transaction(tx_elements, &keychain).unwrap()
}

/// Deterministically generate an output defined by our test scheme
fn test_output(value: u64) -> transaction::Output {
	let keychain = keychain_for_tests();
	let key_id = keychain.derive_key_id(value as u32).unwrap();
	let msg = ProofMessageElements::new(value, &key_id);
	let commit = keychain.commit(value, &key_id).unwrap();
	let proof = proof::create(
		&keychain,
		value,
		&key_id,
		commit,
		None,
		msg.to_proof_message(),
	).unwrap();
	transaction::Output {
		features: transaction::OutputFeatures::DEFAULT_OUTPUT,
		commit: commit,
		proof: proof,
	}
}

/// Deterministically generate a coinbase output defined by our test scheme
fn test_coinbase_output(value: u64) -> transaction::Output {
	let keychain = keychain_for_tests();
	let key_id = keychain.derive_key_id(value as u32).unwrap();
	let msg = ProofMessageElements::new(value, &key_id);
	let commit = keychain.commit(value, &key_id).unwrap();
	let proof = proof::create(
		&keychain,
		value,
		&key_id,
		commit,
		None,
		msg.to_proof_message(),
	).unwrap();
	transaction::Output {
		features: transaction::OutputFeatures::COINBASE_OUTPUT,
		commit: commit,
		proof: proof,
	}
}

fn keychain_for_tests() -> Keychain {
	let seed = "pool_tests";
	let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());
	Keychain::from_seed(seed.as_bytes()).unwrap()
}

/// A generic TxSource representing a test
fn test_source() -> TxSource {
	TxSource {
		debug_name: "test".to_string(),
		identifier: "127.0.0.1".to_string(),
	}
}
