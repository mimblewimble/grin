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

use core::core::hash::{Hash, Hashed};
use core::core::id::ShortIdentifiable;
use core::core::transaction;
use core::core::{Block, Committed, CompactBlock, Transaction};
use keychain::BlindingFactor;
use pool::Pool;
use std::collections::HashMap;
use std::sync::Arc;
use types::*;
use util::LOGGER;
use util::secp::pedersen::Commitment;
use util::{secp_static, static_secp_instance};

/// A minimal (EXPERIMENTAL) transaction pool implementation
pub struct TransactionPool<T> {
	/// Pool Config
	pub config: PoolConfig,

	/// Our transaction pool.
	pub txpool: Pool<T>,
	/// TODO - Our stem transaction pool (just another instance of a pool?)
	pub stempool: Pool<T>,

	/// The blockchain
	pub blockchain: Arc<T>,
	/// The pool adapter
	pub adapter: Arc<PoolAdapter>,

	// TODO - only needed to get dandelion monitor to compile right now...
	pub stem_transactions: HashMap<Hash, Transaction>,
	pub time_stem_transactions: HashMap<Hash, i64>,
}

impl<T> TransactionPool<T>
where
	T: BlockChain,
{
	/// Create a new transaction pool
	pub fn new(config: PoolConfig, chain: Arc<T>, adapter: Arc<PoolAdapter>) -> TransactionPool<T> {
		TransactionPool {
			config: config,
			txpool: Pool::new(chain.clone()),
			stempool: Pool::new(chain.clone()),
			blockchain: chain,
			adapter: adapter,

			stem_transactions: HashMap::new(),
			time_stem_transactions: HashMap::new(),
		}
	}

	// TODO - implement this...
	pub fn deaggregate_and_add_to_memory_pool(
		&mut self,
		source: TxSource,
		tx: Transaction,
		stem: bool,
	) -> Result<(), PoolError> {
		self.add_to_memory_pool(source, tx, stem)
	}

	/// Add a new transaction to the pool.
	/// Validation of the tx (and all txs in the pool) is done via a readonly
	/// txhashset extension. ***EXPERIMENTAL***
	pub fn add_to_memory_pool(
		&mut self,
		src: TxSource,
		tx: Transaction,
		_stem: bool,
	) -> Result<(), PoolError> {
		// Do we have the capacity to accept this transaction?
		self.is_acceptable(&tx)?;

		// Make sure the transaction is valid before anything else.
		tx.validate().map_err(|e| PoolError::InvalidTx(e))?;

		// Attempt to add to the pool (validating against chain state).
		let entry = PoolEntry {
			src,
			// TODO - not using the time yet (for stem pool)
			timer: 0,
			tx: tx.clone(),
		};
		self.txpool.add_to_pool(entry, vec![])?;

		// Notify other parts of the system that we added the tx successfull.
		self.adapter.tx_accepted(&tx);

		Ok(())
	}

	pub fn reconcile_block(&mut self, block: &Block) -> Result<(), PoolError> {
		// First reconcile the txpool.
		self.txpool.reconcile_block(block, None)?;

		// Then reconcile the stempool, accounting for the txpool txs
		let txpool_tx = self.txpool.aggregate_transaction()?;
		self.stempool.reconcile_block(block, Some(&txpool_tx))?;

		Ok(())
	}

	/// TODO - not yet implemented
	pub fn remove_from_stempool(&mut self, tx_hash: &Hash) {
		// TODO - not yet implemented
	}

	pub fn retrieve_transactions(&self, cb: &CompactBlock) -> Vec<Transaction> {
		self.txpool.retrieve_transactions(cb)
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

	/// Get the total size of the pool (regular pool + stem pool).
	pub fn total_size(&self) -> usize {
		self.txpool.size() + self.stempool.size()
	}

	pub fn prepare_mineable_transactions(&self, num_to_fetch: u32) -> Vec<Transaction> {
		self.txpool.prepare_mineable_transactions(num_to_fetch)
	}
}
