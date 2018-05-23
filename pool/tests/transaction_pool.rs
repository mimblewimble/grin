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

use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, RwLock};

use core::core::{Block, BlockHeader, Output, OutputFeatures, OutputIdentifier,
                 ProofMessageElements, Transaction};

use chain::ChainStore;
use chain::store::ChainKVStore;
use chain::txhashset;
use chain::txhashset::TxHashSet;
use core::core::Proof;
use core::core::hash::{Hash, Hashed};
use core::core::pmmr::MerkleProof;
use core::core::target::Difficulty;
use core::global;
use core::global::ChainTypes;
use pool::*;
use types::PoolError::InvalidTx;

use keychain::{BlindingFactor, Keychain};
use util::secp::pedersen::Commitment;
use wallet::libwallet;
use wallet::libwallet::{build, proof, reward};

use pool::TransactionPool;
use pool::types::*;

#[derive(Clone)]
struct ChainAdapter {
	pub txhashset: Arc<RwLock<TxHashSet>>,
	// pub store: Arc<ChainStore>,
}

impl ChainAdapter {
	fn init(db_root: String) -> Result<ChainAdapter, String> {
		let target_dir = format!("target/{}", db_root);
		let chain_store = ChainKVStore::new(target_dir.clone())
			.map_err(|e| format!("failed to init chain_store, {}", e))?;
		let store = Arc::new(chain_store);
		let txhashset = TxHashSet::open(target_dir.clone(), store.clone())
			.map_err(|e| format!("failed to init txhashset, {}", e))?;

		Ok(ChainAdapter {
			txhashset: Arc::new(RwLock::new(txhashset)),
			// store: store.clone(),
		})
	}
}

impl BlockChain for ChainAdapter {
	fn validate_raw_txs(
		&self,
		txs: Vec<Transaction>,
		pre_tx: Option<&Transaction>,
	) -> Result<Vec<Transaction>, PoolError> {
		let height = 1;
		let mut txhashset = self.txhashset.write().unwrap();
		let res = txhashset::extending_readonly(&mut txhashset, |extension| {
			let valid_txs = extension.validate_raw_txs(txs, pre_tx, height)?;
			Ok(valid_txs)
		}).map_err(|e| PoolError::Other(format!("Error: {:?}", e)))?;

		Ok(res)
	}

	// For these tests we just assume coinbase spends have matured sufficiently.
	// We will test the Merkle proof verification logic elsewhere.
	fn verify_coinbase_maturity(&self, _tx: &Transaction) -> Result<(), PoolError> {
		Ok(())
	}
}

fn test_setup(chain: &Arc<ChainAdapter>) -> TransactionPool<ChainAdapter> {
	TransactionPool::new(
		PoolConfig {
			accept_fee_base: 0,
			max_pool_size: 50,
			dandelion_probability: 90,
			dandelion_embargo: 30,
		},
		chain.clone(),
		Arc::new(NoopAdapter {}),
	)
}

/// Deterministically generate an output defined by our test scheme
fn test_output(keychain: &Keychain, value: u64) -> Output {
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

fn test_transaction_spending_coinbase(
	keychain: &Keychain,
	header: &BlockHeader,
	output_values: Vec<u64>,
	txhashset: &mut TxHashSet,
) -> Transaction {
	let output_sum = output_values.iter().sum::<u64>() as i64;

	let fees: i64 = 60_000_000_000 - output_sum;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	// single input spending a single coinbase (deterministic key_id aka height)
	{
		let key_id = keychain.derive_key_id(header.height as u32).unwrap();
		tx_elements.push(build::coinbase_input(
			60_000_000_000,
			header.hash(),
			MerkleProof::default(),
			key_id,
		));
	}

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(build::output(output_value, key_id));
	}

	tx_elements.push(build::with_fee(fees as u64));

	build::transaction(tx_elements, &keychain).unwrap()
}

fn test_transaction(
	keychain: &Keychain,
	input_values: Vec<u64>,
	output_values: Vec<u64>,
) -> Transaction {
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

fn test_source() -> TxSource {
	TxSource {
		debug_name: format!("test"),
		identifier: format!("127.0.0.1"),
	}
}

fn clean_output_dir(db_root: String) {
	let _ = fs::remove_dir_all(format!("target/{}", db_root));
}

/// Test we can add some txs to the pool (both stempool and txpool).
#[test]
fn test_basic_pool_add() {
	let keychain = Keychain::from_random_seed().unwrap();

	let db_root = ".grin_basic_pool_add".to_string();
	clean_output_dir(db_root.clone());

	let chain = ChainAdapter::init(db_root.clone()).unwrap();

	// Initialize the chain/txhashset with a few blocks,
	// so we have a non-empty UTXO set.
	let header = {
		let height = 1;
		let key_id = keychain.derive_key_id(height as u32).unwrap();
		let reward = libwallet::reward::output(&keychain, &key_id, 0, height).unwrap();
		let block = Block::new(&BlockHeader::default(), vec![], Difficulty::one(), reward).unwrap();

		let mut txhashset = chain.txhashset.write().unwrap();
		txhashset::extending(&mut txhashset, |extension| extension.apply_block(&block)).unwrap();

		block.header
	};

	// Initialize a new pool with our chain adapter.
	let pool = RwLock::new(test_setup(&Arc::new(chain.clone())));

	// Now create tx to spend a coinbase, giving us some useful outputs for testing
	// with.
	let initial_tx = {
		let mut txhashset = chain.txhashset.write().unwrap();
		test_transaction_spending_coinbase(&keychain, &header, vec![5, 6, 7, 8], &mut txhashset)
	};

	// Add this tx to the pool (stem=false, direct to txpool).
	{
		let mut write_pool = pool.write().unwrap();
		write_pool
			.add_to_pool(test_source(), initial_tx, false)
			.unwrap();
		assert_eq!(write_pool.total_size(), 1);
	}

	let parent_transaction = test_transaction(&keychain, vec![5, 6], vec![9]);

	// Prepare a second transaction, connected to both previous txs.
	let child_transaction = test_transaction(&keychain, vec![7, 8, 9], vec![12]);

	// Take a write lock and add a couple of entries to the pool.
	{
		let mut write_pool = pool.write().unwrap();

		// Check we have a single initial tx in the pool.
		assert_eq!(write_pool.total_size(), 1);

		// First, add the transaction spending outputs from the initial tx.
		write_pool
			.add_to_pool(test_source(), parent_transaction, true)
			.unwrap();
		assert_eq!(write_pool.total_size(), 2);

		// Now, add another tx spending outputs from the previous tx.
		write_pool
			.add_to_pool(test_source(), child_transaction, true)
			.unwrap();
		assert_eq!(write_pool.total_size(), 3);
	}

	// Now fluff the stempool.
	{
		let mut write_pool = pool.write().unwrap();
		write_pool.fluff_stempool().unwrap();
		assert_eq!(write_pool.total_size(), 2);
	}
}
