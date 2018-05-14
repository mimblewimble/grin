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
use core::core::id::ShortIdentifiable;
use core::core::transaction;
use core::core::{Block, Committed, CompactBlock, Transaction};
use keychain::BlindingFactor;
use types::*;
use util::{secp_static, static_secp_instance};
use util::secp::pedersen::Commitment;
use util::LOGGER;

/// A minimal (EXPERIMENTAL) transaction pool implementation
pub struct MinimalTxPool<T> {
	/// Pool Config
	pub config: PoolConfig,
	/// Transaction in the pool keyed by hash
	// TODO - these need to be Boxed up I think?
	pub transactions: HashMap<Hash, Transaction>,
	pub time_stem_transactions: HashMap<Hash, i64>,
	pub stem_transactions: HashMap<Hash, Transaction>,
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
			stem_transactions: HashMap::new(),
			time_stem_transactions: HashMap::new(),
			tx_insert_order: Vec::new(),
			blockchain: chain,
			adapter: adapter,
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
		self.tx_insert_order.push(tx_hash);
		self.transactions.insert(tx_hash, tx.clone());
		self.adapter.tx_accepted(&tx);

		Ok(())
	}

	// TODO - not implemented
	pub fn reconcile_block(&mut self, block: &Block) -> Result<Vec<Transaction>, PoolError> {
		self.transactions.clear();
		self.tx_insert_order.clear();

		// TODO - need to return the evicted txs (not yet used anywhere though)
		Ok(vec![])
	}

	/// TODO - not yet implemented
	pub fn remove_from_stempool(&mut self, tx_hash: &Hash) {
		// TODO - not yet implemented
	}

	/// TODO - not yet fully implemented
	pub fn prepare_mineable_transactions(
		&self,
		num_to_fetch: u32,
	) -> Vec<Box<transaction::Transaction>> {
		self.transactions
			.values()
			.map(|x| Box::new(x.clone()))
			.collect()
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

	/// Query the tx pool for all known txs based on kernel short_ids
	/// from the provided compact_block.
	/// Note: does not validate that we return the full set of required txs.
	/// The caller will need to validate that themselves.
	pub fn retrieve_transactions(&self, cb: &CompactBlock) -> Vec<Transaction> {
		debug!(
			LOGGER,
			"pool: retrieve_transactions: kern_ids - {:?}, txs - {}, {:?}",
			cb.kern_ids,
			self.transactions.len(),
			self.transactions.keys(),
		);

		let mut txs = vec![];

		for tx in self.transactions.values() {
			for kernel in &tx.kernels {
				// rehash each kernel to calculate the block specific short_id
				let short_id = kernel.short_id(&cb.hash(), cb.nonce);

				// if any kernel matches then keep the tx for later
				if cb.kern_ids.contains(&short_id) {
					txs.push(tx.clone());
					break;
				}
			}
		}

		debug!(
			LOGGER,
			"pool: retrieve_transactions: matching txs from pool - {}",
			txs.len(),
		);

		txs
	}
}
