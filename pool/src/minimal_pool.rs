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
	pub transactions: Vec<Box<Transaction>>,

	// TODO - implement stem functionality
	pub time_stem_transactions: HashMap<Hash, i64>,
	pub stem_transactions: HashMap<Hash, Transaction>,

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
			transactions: Vec::new(),
			stem_transactions: HashMap::new(),
			time_stem_transactions: HashMap::new(),
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
		// Do we have the capacity to accept this transaction?
		self.is_acceptable(&tx)?;

		// Make sure the transaction is valid before anything else.
		tx.validate().map_err(|e| PoolError::InvalidTx(e))?;

		// Aggregate this new tx with all existing txs in the pool.
		let mut txs = self.transactions
			.iter()
			.map(|x| *x.clone())
			.collect::<Vec<_>>();
		txs.push(tx.clone());
		let agg_tx =
			transaction::aggregate_with_cut_through(txs).map_err(|_| PoolError::GenericPoolError)?;

		// Validate aggregated tx against the chain txhashset extension.
		self.blockchain.validate_raw_tx(&agg_tx)?;
		self.transactions.push(Box::new(tx.clone()));
		self.adapter.tx_accepted(&tx);

		Ok(())
	}

	// TODO - pass vec of txs around? Then we can build it incrementally?
	// Aggregate this new tx with all existing txs in the pool.
	// Skip some validation steps as the tx was already in our tx pool.
	fn internal_add_to_memory_pool(&mut self, tx: Transaction) -> Result<(), PoolError> {
		let mut txs = self.transactions
			.iter()
			.map(|x| *x.clone())
			.collect::<Vec<_>>();
		txs.push(tx.clone());
		let agg_tx =
			transaction::aggregate_with_cut_through(txs).map_err(|_| PoolError::GenericPoolError)?;

		// Validate aggregated tx against the chain txhashset extension.
		self.blockchain.validate_raw_tx(&agg_tx)?;
		self.transactions.push(Box::new(tx.clone()));

		Ok(())
	}

	// TODO - not fully implemented
	pub fn reconcile_block(&mut self, block: &Block) -> Result<Vec<Transaction>, PoolError> {
		let mut candidate_transactions = self.transactions.clone();

		// Simple check comparing tx kernels against block kernels
		// This covers majority of the simple case?
		candidate_transactions.retain(|tx| {
			let mut keep = true;
			for k in &tx.kernels {
				if block.kernels.contains(&k) {
					keep = false;
					break;
				}
			}
			keep
		});

		debug!(
			LOGGER,
			"pool: reconcile_block: txs - {}, candidate txs - {}",
			self.transactions.len(),
			candidate_transactions.len(),
		);

		// Clear the tx pool as we are about to start adding valid txs back in.
		self.transactions.clear();

		if candidate_transactions.is_empty() {
			debug!(LOGGER, "pool: reconcile_block: pool empty! Done.");
			assert!(self.transactions.is_empty());
			return Ok(vec![]);
		}

		// Initial quick check will catch many simple cases.
		// Aggregate everything remaining in the pool and check if it validates.
		// If we validate successfully here then we are done.
		{
			let txs = candidate_transactions.iter().map(|x| *x.clone()).collect();
			let agg_tx = transaction::aggregate_with_cut_through(txs)
				.map_err(|_| PoolError::GenericPoolError)?;
			if let Ok(_) = self.blockchain.validate_raw_tx(&agg_tx) {
				debug!(
					LOGGER,
					"pool: reconcile_block: candidate txs fully validate. Done."
				);
				self.transactions = candidate_transactions;
				return Ok(vec![]);
			}
		}

		// TODO - add back all the fully unspent ones as a shortcut?

		// Naive but robust - add everything back sequentially,
		// validate the pool each time.
		for tx in candidate_transactions {
			self.internal_add_to_memory_pool(*tx);
		}

		// TODO - need to return the evicted txs (not yet used anywhere though)
		Ok(vec![])
	}

	/// TODO - not yet implemented
	pub fn remove_from_stempool(&mut self, tx_hash: &Hash) {
		// TODO - not yet implemented
	}

	/// TODO - not yet fully implemented
	pub fn prepare_mineable_transactions(&self, num_to_fetch: u32) -> Vec<Box<Transaction>> {
		self.transactions.clone()
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
			"pool: retrieve_transactions: kern_ids - {:?}, txs - {}",
			cb.kern_ids,
			self.transactions.len(),
		);

		let mut txs = vec![];

		for tx in &self.transactions {
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

		txs.into_iter().map(|x| *x).collect()
	}
}
