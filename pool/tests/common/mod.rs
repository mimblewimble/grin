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

//! Common test functions

extern crate blake2_rfc as blake2;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_pool as pool;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

extern crate rand;
extern crate chrono;

use std::fs;
use std::sync::{Arc, RwLock};

use core::core::{BlockHeader, Transaction};

use chain::store::ChainStore;
use chain::txhashset;
use chain::txhashset::TxHashSet;
use core::core::hash::Hashed;
use core::core::merkle_proof::MerkleProof;
use pool::*;

use keychain::Keychain;
use wallet::libtx;

use pool::TransactionPool;
use pool::types::*;

#[derive(Clone)]
pub struct ChainAdapter {
	pub txhashset: Arc<RwLock<TxHashSet>>,
	pub store: Arc<ChainStore>,
}

impl ChainAdapter {
	pub fn init(db_root: String) -> Result<ChainAdapter, String> {
		let target_dir = format!("target/{}", db_root);
		let db_env = Arc::new(store::new_env(target_dir.clone()));
		let chain_store =
			ChainStore::new(db_env).map_err(|e| format!("failed to init chain_store, {:?}", e))?;
		let store = Arc::new(chain_store);
		let txhashset = TxHashSet::open(target_dir.clone(), store.clone(), None)
			.map_err(|e| format!("failed to init txhashset, {}", e))?;

		Ok(ChainAdapter {
			txhashset: Arc::new(RwLock::new(txhashset)),
			store: store.clone(),
		})
	}
}

impl BlockChain for ChainAdapter {
	fn validate_raw_txs(
		&self,
		txs: Vec<Transaction>,
		pre_tx: Option<Transaction>,
	) -> Result<Vec<Transaction>, PoolError> {
		let mut txhashset = self.txhashset.write().unwrap();
		let res = txhashset::extending_readonly(&mut txhashset, |extension| {
			let valid_txs = extension.validate_raw_txs(txs, pre_tx)?;
			Ok(valid_txs)
		}).map_err(|e| PoolError::Other(format!("Error: test chain adapter: {:?}", e)))?;
		Ok(res)
	}

	// Mocking this check out for these tests.
	// We will test the Merkle proof verification logic elsewhere.
	fn verify_coinbase_maturity(&self, _tx: &Transaction) -> Result<(), PoolError> {
		Ok(())
	}

	// Mocking this out for these tests.
	fn verify_tx_lock_height(&self, _tx: &Transaction) -> Result<(), PoolError> {
		Ok(())
	}
}

pub fn test_setup(chain: &Arc<ChainAdapter>) -> TransactionPool<ChainAdapter> {
	TransactionPool::new(
		PoolConfig {
			accept_fee_base: 0,
			max_pool_size: 50,
		},
		chain.clone(),
		Arc::new(NoopAdapter {}),
	)
}

pub fn test_transaction_spending_coinbase<K>(
	keychain: &K,
	header: &BlockHeader,
	output_values: Vec<u64>,
) -> Transaction
where
	K: Keychain,
{
	let output_sum = output_values.iter().sum::<u64>() as i64;

	let coinbase_reward: u64 = 60_000_000_000;

	let fees: i64 = coinbase_reward as i64 - output_sum;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	// single input spending a single coinbase (deterministic key_id aka height)
	{
		let key_id = keychain.derive_key_id(header.height as u32).unwrap();
		tx_elements.push(libtx::build::coinbase_input(
			coinbase_reward,
			key_id,
		));
	}

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(libtx::build::output(output_value, key_id));
	}

	tx_elements.push(libtx::build::with_fee(fees as u64));

	libtx::build::transaction(tx_elements, keychain).unwrap()
}

pub fn test_transaction<K>(
	keychain: &K,
	input_values: Vec<u64>,
	output_values: Vec<u64>,
) -> Transaction
where
	K: Keychain,
{
	let input_sum = input_values.iter().sum::<u64>() as i64;
	let output_sum = output_values.iter().sum::<u64>() as i64;

	let fees: i64 = input_sum - output_sum;
	assert!(fees >= 0);

	let mut tx_elements = Vec::new();

	for input_value in input_values {
		let key_id = keychain.derive_key_id(input_value as u32).unwrap();
		tx_elements.push(libtx::build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = keychain.derive_key_id(output_value as u32).unwrap();
		tx_elements.push(libtx::build::output(output_value, key_id));
	}
	tx_elements.push(libtx::build::with_fee(fees as u64));

	libtx::build::transaction(tx_elements, keychain).unwrap()
}

pub fn test_source() -> TxSource {
	TxSource {
		debug_name: format!("test"),
		identifier: format!("127.0.0.1"),
	}
}

pub fn clean_output_dir(db_root: String) {
	if let Err(e) = fs::remove_dir_all(format!("target/{}", db_root)) {
		println!("cleaning output dir failed - {:?}", e)
	}
}
