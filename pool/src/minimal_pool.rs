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

//! A minimal (EXPERIMENTAL) transaction pool implementation

use std::collections::HashMap;
use std::sync::Arc;
use core::core::hash::{Hash, Hashed};
use core::core::transaction;
use core::core::{Committed, Transaction};
use keychain::BlindingFactor;
use types::*;
use util::{secp_static, static_secp_instance};
use util::secp::pedersen::Commitment;

/// A minimal (EXPERIMENTAL) transaction pool implementation
pub struct MinimalTxPool<T> {
	/// Pool Config
	pub config: PoolConfig,
	/// Transaction in the pool keyed by hash
	pub transactions: HashMap<Hash, Transaction>,
	/// Transaction hashes in the order they were added to the pool
	pub tx_insert_order: Vec<Hash>,
	/// The blockchain
	pub blockchain: Arc<T>,
	/// The pool adapter
	pub adapter: Arc<PoolAdapter>,
}

impl<T> MinimalTxPool<T>
where
	T: BlockChain,
{
	/// Create a new transaction pool
	pub fn new(config: PoolConfig, chain: Arc<T>, adapter: Arc<PoolAdapter>) -> MinimalTxPool<T> {
		MinimalTxPool {
			config: config,
			transactions: HashMap::new(),
			tx_insert_order: Vec::new(),
			blockchain: chain,
			adapter: adapter,
		}
	}

	/// Add a new transaction to the pool.
	/// Validation of the tx (and all txs in the pool) is done via a readonly txhashset extension.
	/// ***EXPERIMENTAL***
	pub fn add_to_memory_pool(
		&mut self,
		_: TxSource,
		tx: Transaction,
		_stem: bool,
	) -> Result<(), PoolError> {
		let tx_hash = tx.hash();

		// Do we have the capacity to accept this transaction?
		self.is_acceptable(&tx)?;

		// Make sure the transaction is valid before anything else.
		tx.validate().map_err(|e| PoolError::InvalidTx(e))?;

		// Aggregate this new tx with all existing txs in the pool.
		// Consider "caching" this aggregated tx in the pool somehow?
		// TODO - fix the excessive cloning() here somehow
		let mut txs = self.transactions.values().cloned().collect::<Vec<_>>();
		txs.push(tx.clone());
		let agg_tx =
			transaction::aggregate_with_cut_through(txs).map_err(|_| PoolError::GenericPoolError)?;

		// Validate aggregated tx against the chain txhashset extension.
		self.blockchain.validate_raw_tx(&agg_tx)?;
		self.transactions.insert(tx_hash, tx);

		Ok(())
	}

	/// Whether the transaction is acceptable to the pool, given both how
	/// full the pool is and the transaction weight.
	fn is_acceptable(&self, tx: &Transaction) -> Result<(), PoolError> {
		if self.total_size() > self.config.max_pool_size {
			// TODO evict old/large transactions instead
			return Err(PoolError::OverCapacity);
		}

		// for a basic transaction (1 input, 2 outputs) -
		// (-1 * 1) + (4 * 2) + 1 = 8
		// 8 * 10 = 80
		if self.config.accept_fee_base > 0 {
			let threshold = (tx.tx_weight() as u64) * self.config.accept_fee_base;
			if tx.fee() < threshold {
				return Err(PoolError::LowFeeTransaction(threshold));
			}
		}
		Ok(())
	}

	/// Get the total size of the pool.
	pub fn total_size(&self) -> usize {
		self.transactions.len()
	}
}
