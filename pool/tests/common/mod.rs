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

extern crate chrono;
extern crate rand;

use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, RwLock};

use core::core::hash::{Hash, Hashed};
use core::core::verifier_cache::VerifierCache;
use core::core::{Block, BlockHeader, BlockSums, Committed, Transaction};

use chain::store::ChainStore;
use chain::types::Tip;
use pool::*;

use keychain::{ExtKeychain, Keychain};
use wallet::libtx;

use pool::types::*;
use pool::TransactionPool;
use util::secp::pedersen::Commitment;

#[derive(Clone)]
pub struct ChainAdapter {
	pub store: Arc<ChainStore>,
	pub utxo: Arc<RwLock<HashSet<Commitment>>>,
}

impl ChainAdapter {
	pub fn init(db_root: String) -> Result<ChainAdapter, String> {
		let target_dir = format!("target/{}", db_root);
		let db_env = Arc::new(store::new_env(target_dir.clone()));
		let chain_store =
			ChainStore::new(db_env).map_err(|e| format!("failed to init chain_store, {:?}", e))?;
		let store = Arc::new(chain_store);
		let utxo = Arc::new(RwLock::new(HashSet::new()));

		Ok(ChainAdapter { store, utxo })
	}

	pub fn update_db_for_block(&self, block: &Block) {
		let header = &block.header;
		let batch = self.store.batch().unwrap();
		let tip = Tip::from_block(&header);
		batch.save_block_header(&header).unwrap();
		batch.save_head(&tip).unwrap();

		// Retrieve previous block_sums from the db.
		let prev_sums = if let Ok(prev_sums) = batch.get_block_sums(&header.previous) {
			prev_sums
		} else {
			BlockSums::default()
		};

		// Overage is based purely on the new block.
		// Previous block_sums have taken all previous overage into account.
		let overage = header.overage();

		// Offset on the other hand is the total kernel offset from the new block.
		let offset = header.total_kernel_offset();

		// Verify the kernel sums for the block_sums with the new block applied.
		let (utxo_sum, kernel_sum) = (prev_sums, block as &Committed)
			.verify_kernel_sums(overage, offset)
			.unwrap();

		let block_sums = BlockSums {
			utxo_sum,
			kernel_sum,
		};
		batch.save_block_sums(&header.hash(), &block_sums).unwrap();

		batch.commit().unwrap();

		{
			let mut utxo = self.utxo.write().unwrap();
			for x in block.inputs() {
				utxo.remove(&x.commitment());
			}
			for x in block.outputs() {
				utxo.insert(x.commitment());
			}
		}
	}
}

impl BlockChain for ChainAdapter {
	fn chain_head(&self) -> Result<BlockHeader, PoolError> {
		self.store
			.head_header()
			.map_err(|_| PoolError::Other(format!("failed to get chain head")))
	}

	fn get_block_header(&self, hash: &Hash) -> Result<BlockHeader, PoolError> {
		self.store
			.get_block_header(hash)
			.map_err(|_| PoolError::Other(format!("failed to get block header")))
	}

	fn get_block_sums(&self, hash: &Hash) -> Result<BlockSums, PoolError> {
		self.store
			.get_block_sums(hash)
			.map_err(|_| PoolError::Other(format!("failed to get block sums")))
	}

	fn validate_tx(&self, tx: &Transaction) -> Result<(), pool::PoolError> {
		let utxo = self.utxo.read().unwrap();

		for x in tx.outputs() {
			if utxo.contains(&x.commitment()) {
				return Err(PoolError::Other(format!("output commitment not unique")));
			}
		}

		for x in tx.inputs() {
			if !utxo.contains(&x.commitment()) {
				return Err(PoolError::Other(format!("not in utxo set")));
			}
		}

		Ok(())
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

pub fn test_setup(
	chain: Arc<BlockChain>,
	verifier_cache: Arc<RwLock<VerifierCache>>,
) -> TransactionPool {
	TransactionPool::new(
		PoolConfig {
			accept_fee_base: 0,
			max_pool_size: 50,
		},
		chain.clone(),
		verifier_cache.clone(),
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
		let key_id = ExtKeychain::derive_key_id(1, header.height as u32, 0, 0, 0);
		tx_elements.push(libtx::build::coinbase_input(coinbase_reward, key_id));
	}

	for output_value in output_values {
		let key_id = ExtKeychain::derive_key_id(1, output_value as u32, 0, 0, 0);
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
		let key_id = ExtKeychain::derive_key_id(1, input_value as u32, 0, 0, 0);
		tx_elements.push(libtx::build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = ExtKeychain::derive_key_id(1, output_value as u32, 0, 0, 0);
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
