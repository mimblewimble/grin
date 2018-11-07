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

use std::collections::VecDeque;
use std::sync::Arc;
use util::RwLock;

use chrono::prelude::Utc;

use core::core::hash::{Hash, Hashed};
use core::core::id::ShortId;
use core::core::verifier_cache::VerifierCache;
use core::core::{transaction, Block, BlockHeader, Transaction};
use pool::Pool;
use types::{BlockChain, PoolAdapter, PoolConfig, PoolEntry, PoolEntryState, PoolError, TxSource};

// Cache this many txs to handle a potential fork and re-org.
const REORG_CACHE_SIZE: usize = 100;

/// Transaction pool implementation.
pub struct TransactionPool {
	/// Pool Config
	pub config: PoolConfig,
	/// Our transaction pool.
	pub txpool: Pool,
	/// Our Dandelion "stempool".
	pub stempool: Pool,
	/// Cache of previous txs in case of a re-org.
	pub reorg_cache: Arc<RwLock<VecDeque<PoolEntry>>>,
	/// The blockchain
	pub blockchain: Arc<BlockChain>,
	pub verifier_cache: Arc<RwLock<VerifierCache>>,
	/// The pool adapter
	pub adapter: Arc<PoolAdapter>,
}

impl TransactionPool {
	/// Create a new transaction pool
	pub fn new(
		config: PoolConfig,
		chain: Arc<BlockChain>,
		verifier_cache: Arc<RwLock<VerifierCache>>,
		adapter: Arc<PoolAdapter>,
	) -> TransactionPool {
		TransactionPool {
			config,
			txpool: Pool::new(chain.clone(), verifier_cache.clone(), "txpool".to_string()),
			stempool: Pool::new(
				chain.clone(),
				verifier_cache.clone(),
				"stempool".to_string(),
			),
			reorg_cache: Arc::new(RwLock::new(VecDeque::new())),
			blockchain: chain,
			verifier_cache,
			adapter,
		}
	}

	pub fn chain_head(&self) -> Result<BlockHeader, PoolError> {
		self.blockchain.chain_head()
	}

	fn add_to_stempool(&mut self, entry: PoolEntry, header: &BlockHeader) -> Result<(), PoolError> {
		// Add tx to stempool (passing in all txs from txpool to validate against).
		self.stempool
			.add_to_pool(entry, self.txpool.all_transactions(), header)?;

		// Note: we do not notify the adapter here,
		// we let the dandelion monitor handle this.
		Ok(())
	}

	fn add_to_reorg_cache(&mut self, entry: PoolEntry) -> Result<(), PoolError> {
		let mut cache = self.reorg_cache.write();
		cache.push_back(entry);
		if cache.len() > REORG_CACHE_SIZE {
			cache.pop_front();
		}
		debug!("added tx to reorg_cache: size now {}", cache.len());
		Ok(())
	}

	fn add_to_txpool(
		&mut self,
		mut entry: PoolEntry,
		header: &BlockHeader,
	) -> Result<(), PoolError> {
		// First deaggregate the tx based on current txpool txs.
		if entry.tx.kernels().len() > 1 {
			let txs = self.txpool.find_matching_transactions(entry.tx.kernels());
			if !txs.is_empty() {
				let tx = transaction::deaggregate(entry.tx, txs)?;
				tx.validate(self.verifier_cache.clone())?;
				entry.tx = tx;
				entry.src.debug_name = "deagg".to_string();
			}
		}
		self.txpool.add_to_pool(entry.clone(), vec![], header)?;

		// We now need to reconcile the stempool based on the new state of the txpool.
		// Some stempool txs may no longer be valid and we need to evict them.
		{
			let txpool_tx = self.txpool.aggregate_transaction()?;
			self.stempool.reconcile(txpool_tx, header)?;
		}

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
		header: &BlockHeader,
	) -> Result<(), PoolError> {
		// Quick check to deal with common case of seeing the *same* tx
		// broadcast from multiple peers simultaneously.
		if !stem && self.txpool.contains_tx(tx.hash()) {
			return Err(PoolError::DuplicateTx);
		}

		// Do we have the capacity to accept this transaction?
		self.is_acceptable(&tx, stem)?;

		// Make sure the transaction is valid before anything else.
		tx.validate(self.verifier_cache.clone())
			.map_err(PoolError::InvalidTx)?;

		// Check the tx lock_time is valid based on current chain state.
		self.blockchain.verify_tx_lock_height(&tx)?;

		// Check coinbase maturity before we go any further.
		self.blockchain.verify_coinbase_maturity(&tx)?;

		let entry = PoolEntry {
			state: PoolEntryState::Fresh,
			src,
			tx_at: Utc::now(),
			tx,
		};

		if stem {
			// TODO - what happens to txs in the stempool in a re-org scenario?
			self.add_to_stempool(entry, header)?;
		} else {
			self.add_to_txpool(entry.clone(), header)?;
			self.add_to_reorg_cache(entry)?;
		}
		Ok(())
	}

	fn reconcile_reorg_cache(&mut self, header: &BlockHeader) -> Result<(), PoolError> {
		let entries = self.reorg_cache.read().iter().cloned().collect::<Vec<_>>();
		debug!("reconcile_reorg_cache: size: {} ...", entries.len());
		for entry in entries {
			let _ = &self.add_to_txpool(entry.clone(), header);
		}
		debug!("reconcile_reorg_cache: ... done.");
		Ok(())
	}

	/// Reconcile the transaction pool (both txpool and stempool) against the
	/// provided block.
	pub fn reconcile_block(&mut self, block: &Block) -> Result<(), PoolError> {
		// First reconcile the txpool.
		self.txpool.reconcile_block(block);
		self.txpool.reconcile(None, &block.header)?;

		// Take our "reorg_cache" and see if this block means
		// we need to (re)add old txs due to a fork and re-org.
		self.reconcile_reorg_cache(&block.header)?;

		// Now reconcile our stempool, accounting for the updated txpool txs.
		self.stempool.reconcile_block(block);
		{
			let txpool_tx = self.txpool.aggregate_transaction()?;
			self.stempool.reconcile(txpool_tx, &block.header)?;
		}

		Ok(())
	}

	/// Retrieve individual transaction for the given kernel hash.
	pub fn retrieve_tx_by_kernel_hash(&self, hash: Hash) -> Option<Transaction> {
		self.txpool.retrieve_tx_by_kernel_hash(hash)
	}

	/// Retrieve all transactions matching the provided "compact block"
	/// based on the kernel set.
	/// Note: we only look in the txpool for this (stempool is under embargo).
	pub fn retrieve_transactions(
		&self,
		hash: Hash,
		nonce: u64,
		kern_ids: &[ShortId],
	) -> (Vec<Transaction>, Vec<ShortId>) {
		self.txpool.retrieve_transactions(hash, nonce, kern_ids)
	}

	/// Whether the transaction is acceptable to the pool, given both how
	/// full the pool is and the transaction weight.
	fn is_acceptable(&self, tx: &Transaction, stem: bool) -> Result<(), PoolError> {
		if self.total_size() > self.config.max_pool_size {
			// TODO evict old/large transactions instead
			return Err(PoolError::OverCapacity);
		}

		// Check that the stempool can accept this transaction
		if stem {
			if self.stempool.size() > self.config.max_stempool_size {
				// TODO evict old/large transactions instead
				return Err(PoolError::OverCapacity);
			}
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
	pub fn prepare_mineable_transactions(&self) -> Result<Vec<Transaction>, PoolError> {
		self.txpool.prepare_mineable_transactions()
	}
}
