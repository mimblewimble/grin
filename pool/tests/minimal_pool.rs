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
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_pool as pool;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate rand;
extern crate time;

use std::collections::HashMap;

use core::core::{BlockHeader, Output, OutputFeatures, OutputIdentifier, ProofMessageElements,
                 Transaction};

use pool::*;
use core::global;
use blockchain::{DummyChain, DummyChainImpl, DummyOutputSet};
use std::sync::{Arc, RwLock};
use core::global::ChainTypes;
use core::core::Proof;
use core::core::hash::{Hash, Hashed};
use core::core::pmmr::MerkleProof;
use core::core::target::Difficulty;
use types::PoolError::InvalidTx;

use keychain::{BlindingFactor, Keychain};
use wallet::libwallet::{build, proof, reward};

use pool::minimal_pool::{MinimalTxPool, TxPoolSums};
use pool::types::*;
use util::secp_static;

// macro_rules! expect_output_parent {
// 	($pool:expr, $expected:pat, $( $output:expr ),+ ) => {
// 		$(
// 			match $pool
// 			.search_for_best_output(
// 				&OutputIdentifier::from_output(&test_output($output))
// 			) {
// 				$expected => {},
// 				x => panic!(
// 					"Unexpected result from output search for {:?}, got {:?}",
// 					$output,
// 					x,
// 				),
// 			};
// 		)*
// 	}
// }

fn test_setup(chain: &Arc<DummyChainImpl>) -> MinimalTxPool<DummyChainImpl> {
	MinimalTxPool {
		config: PoolConfig {
			accept_fee_base: 0,
			max_pool_size: 50,
			dandelion_probability: 90,
			dandelion_embargo: 30,
		},
		pool_sums: TxPoolSums::default(),
		transactions: HashMap::new(),
		tx_insert_order: Vec::new(),
		blockchain: chain.clone(),
		adapter: Arc::new(NoopAdapter {}),
	}
}

/// Deterministically generate an output defined by our test scheme
fn test_output(value: u64) -> Output {
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
	Output {
		features: OutputFeatures::DEFAULT_OUTPUT,
		commit: commit,
		proof: proof,
	}
}

fn test_transaction(input_values: Vec<u64>, output_values: Vec<u64>) -> Transaction {
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

fn init_pool_sums(values: Vec<u64>) -> TxPoolSums {
	let keychain = keychain_for_tests();

	let output_sums = values
		.iter()
		.cloned()
		.map(|x| {
			let key_id = keychain.derive_key_id(x as u32).unwrap();
			keychain.commit(x, &key_id).unwrap()
		})
		.collect::<Vec<_>>();

	// TODO - what do we do with the kernel_sum here on init???
	// kernel_excess is a commit to what? which is 0 initially?

	let total = values.iter().sum();
	let key_id = keychain.derive_key_id(total as u32).unwrap();
	let kernel_sum = keychain.commit(0, &key_id).unwrap();

	let output_sum = keychain.secp().commit_sum(output_sums, vec![]).unwrap();

	let zero_commit = secp_static::commit_to_zero_value();
	TxPoolSums {
		output_sum,
		kernel_sum,
		offset_sum: BlindingFactor::zero(),
	}
}

// A deterministic keychain.
fn keychain_for_tests() -> Keychain {
	let seed = "minimal_pool_tests";
	let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());
	Keychain::from_seed(seed.as_bytes()).unwrap()
}

fn test_source() -> TxSource {
	TxSource {
		debug_name: format!("test"),
		identifier: format!("127.0.0.1"),
	}
}

/// Add a couple of transactions to the pool.
#[test]
fn test_minimal_basic_pool_add() {
	let mut dummy_chain = DummyChainImpl::new();
	let head_header = BlockHeader {
		height: 1,
		..BlockHeader::default()
	};
	dummy_chain.store_head_header(&head_header);

	// To mirror how this construction is intended to be used
	// the pool is placed inside a RwLock.
	let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

	// Initialize the pool sums so we have some outputs that can be spent.
	{
		let mut write_pool = pool.write().unwrap();
		write_pool.pool_sums = init_pool_sums(vec![5, 6, 7]);
	}

	let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);

	// Prepare a second transaction, connected to the first.
	// TODO - how is this still validating???
	let child_transaction = test_transaction(vec![100], vec![12]);
	// let child_transaction = test_transaction(vec![11, 3], vec![12]);

	// Take the write lock and add a pool entry
	{
		let mut write_pool = pool.write().unwrap();
		assert_eq!(write_pool.total_size(), 0);

		// First, add the transaction rooted in the blockchain
		write_pool
			.add_to_memory_pool(test_source(), parent_transaction, false)
			.unwrap();
		assert_eq!(write_pool.total_size(), 1);

		// Now, add the transaction connected as a child to the first
		write_pool
			.add_to_memory_pool(test_source(), child_transaction, false)
			.unwrap();
		assert_eq!(write_pool.total_size(), 2);
		println!("***** {:?}", write_pool.pool_sums);
	}

	panic!("[wip]");

	// // Now take the read lock and use a few exposed methods to check consistency
	// {
	// 	let read_pool = pool.read().unwrap();
	// 	assert_eq!(read_pool.total_size(), 2);
	// 	expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
	// 	expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
	// 	expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
	// 	expect_output_parent!(read_pool, Parent::Unknown, 20);
	// }
}
