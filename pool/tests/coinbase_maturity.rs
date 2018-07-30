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
extern crate chrono;

pub mod common;

use std::sync::{Arc, RwLock};

use common::{test_source, test_transaction};
use core::core::Transaction;
use keychain::{ExtKeychain, Keychain};
use pool::TransactionPool;
use pool::types::{BlockChain, NoopAdapter, PoolConfig, PoolError};

pub fn test_setup(
	chain: &Arc<CoinbaseMaturityErrorChainAdapter>,
) -> TransactionPool<CoinbaseMaturityErrorChainAdapter> {
	TransactionPool::new(
		PoolConfig {
			accept_fee_base: 0,
			max_pool_size: 50,
		},
		chain.clone(),
		Arc::new(NoopAdapter {}),
	)
}

#[derive(Clone)]
pub struct CoinbaseMaturityErrorChainAdapter {}

impl CoinbaseMaturityErrorChainAdapter {
	pub fn new() -> CoinbaseMaturityErrorChainAdapter {
		CoinbaseMaturityErrorChainAdapter {}
	}
}

impl BlockChain for CoinbaseMaturityErrorChainAdapter {
	fn validate_raw_txs(
		&self,
		_txs: Vec<Transaction>,
		_pre_tx: Option<Transaction>,
	) -> Result<Vec<Transaction>, PoolError> {
		Err(PoolError::Other(
			"not implemented, not a real chain adapter...".to_string(),
		))
	}

	// Returns an ImmatureCoinbase for every tx we pass in.
	fn verify_coinbase_maturity(&self, _tx: &Transaction) -> Result<(), PoolError> {
		Err(PoolError::ImmatureCoinbase)
	}

	// Mocking this out for these tests.
	fn verify_tx_lock_height(&self, _tx: &Transaction) -> Result<(), PoolError> {
		Ok(())
	}
}

/// Test we correctly verify coinbase maturity when adding txs to the pool.
#[test]
fn test_coinbase_maturity() {
	let keychain: ExtKeychain = Keychain::from_random_seed().unwrap();

	// Mocking this up with an adapter that will raise an error for coinbase
	// maturity.
	let chain = CoinbaseMaturityErrorChainAdapter::new();
	let pool = RwLock::new(test_setup(&Arc::new(chain.clone())));

	{
		let mut write_pool = pool.write().unwrap();
		let tx = test_transaction(&keychain, vec![50], vec![49]);
		match write_pool.add_to_pool(test_source(), tx.clone(), true) {
			Err(PoolError::ImmatureCoinbase) => {}
			_ => panic!("Expected an immature coinbase error here."),
		}
	}
}
