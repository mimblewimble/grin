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

//! Transaction pool implementation leveraging txhashset for chain state
//! validation. It is a valid operation to add a tx to the tx pool if the
//! resulting tx pool can be added to the current chain state to produce a
//! valid chain state.

use std::sync::Arc;
use time;

use core::core::transaction;
use core::core::{Block, CompactBlock, Transaction};
use pool::Pool;
use types::*;

/// Transaction pool implementation.
pub struct TransactionPool<T> {
	/// Pool Config
	pub config: PoolConfig,

	/// Our transaction pool.
	pub txpool: Pool<T>,
	/// Our Dandelion "stempool".
	pub stempool: Pool<T>,

	/// The blockchain
	pub blockchain: Arc<T>,
	/// The pool adapter
	pub adapter: Arc<PoolAdapter>,
}

impl<T> TransactionPool<T>
where
	T: BlockChain,
{
	/// Create a new transaction pool
	pub fn new(config: PoolConfig, chain: Arc<T>, adapter: Arc<PoolAdapter>) -> TransactionPool<T> {
		TransactionPool {
			config: config,
			txpool: Pool::new(chain.clone(), format!("txpool")),
			stempool: Pool::new(chain.clone(), format!("stempool")),
			blockchain: chain,
			adapter: adapter,
		}
	}

	fn add_to_stempool(&mut self, entry: PoolEntry) -> Result<(), PoolError> {
		// Add tx to stempool (passing in all txs from txpool to validate against).
		self.stempool
			.add_to_pool(entry.clone(), self.txpool.all_transactions())?;

		// Note: we do not notify the adapter here,
		// we let the dandelion monitor handle this.
		Ok(())
	}

	fn add_to_txpool(&mut self, mut entry: PoolEntry) -> Result<(), PoolError> {
		// First deaggregate the tx based on current txpool txs.
		if entry.tx.kernels.len() > 1 {
			let txs = self.txpool
				.find_matching_transactions(entry.tx.kernels.clone());
			if !txs.is_empty() {
				entry.tx = transaction::deaggregate(entry.tx, txs)?;
				entry.src.debug_name = "deagg".to_string();
			}
		}
		self.txpool.add_to_pool(entry.clone(), vec![])?;

		// We now need to reconcile the stempool based on the new state of the txpool.
		// Some stempool txs may no longer be valid and we need to evict them.
		let txpool_tx = self.txpool.aggregate_transaction()?;
		self.stempool.reconcile(txpool_tx)?;

		self.adapter.tx_accepted(&entry.tx);
		Ok(())
	}

	/// Add the given tx to the pool, directing it to either the stempool or
	/// txpool based on stem flag provided.
	pub fn add_to_pool(
		&mut self,
		src: TxSource,
		tx: Transaction,
		stem: bool,
	) -> Result<(), PoolError> {
		// Do we have the capacity to accept this transaction?
		self.is_acceptable(&tx)?;

		// Make sure the transaction is valid before anything else.
		tx.validate().map_err(|e| PoolError::InvalidTx(e))?;

		// Check the tx lock_time is valid based on current chain state.
		self.blockchain.verify_tx_lock_height(&tx)?;

		// Check coinbase maturity before we go any further.
		self.blockchain.verify_coinbase_maturity(&tx)?;

		let entry = PoolEntry {
			state: PoolEntryState::Fresh,
			src,
			tx_at: time::now_utc().to_timespec(),
			tx: tx.clone(),
		};

		if stem {
			self.add_to_stempool(entry)?;
		} else {
			self.add_to_txpool(entry)?;
		}
		Ok(())
	}

	/// Reconcile the transaction pool (both txpool and stempool) against the
	/// provided block.
	pub fn reconcile_block(&mut self, block: &Block) -> Result<(), PoolError> {
		// First reconcile the txpool.
		self.txpool.reconcile_block(block)?;
		self.txpool.reconcile(None)?;

		// Then reconcile the stempool, accounting for the txpool txs.
		let txpool_tx = self.txpool.aggregate_transaction()?;
		self.stempool.reconcile_block(block)?;
		self.stempool.reconcile(txpool_tx)?;

		Ok(())
	}

	/// Retrieve all transactions matching the provided "compact block"
	/// based on the kernel set.
	/// Note: we only look in the txpool for this (stempool is under embargo).
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

	/// Get the total size of the pool.
	/// Note: we only consider the txpool here as stempool is under embargo.
	pub fn total_size(&self) -> usize {
		self.txpool.size()
	}

	/// Returns a vector of transactions from the txpool so we can build a
	/// block from them.
	pub fn prepare_mineable_transactions(&self, num_to_fetch: u32) -> Vec<Transaction> {
		self.txpool.prepare_mineable_transactions(num_to_fetch)
	}
}
