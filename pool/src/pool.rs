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

pub struct Pool<T> {
	/// Transaction in the pool in simple insertion order.
	pub transactions: Vec<Box<Transaction>>,
	/// The blockchain
	pub blockchain: Arc<T>,
}

impl<T> Pool<T>
where
	T: BlockChain,
{
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

	/// TODO - not yet fully implemented
	pub fn prepare_mineable_transactions(&self, num_to_fetch: u32) -> Vec<Box<Transaction>> {
		self.transactions.clone()
	}

	// Aggregate this new tx with all existing txs in the pool.
	// If we can validate the aggregated tx against the current chain state
	// then we can safely add the tx to the pool.
	pub fn add_to_pool(&mut self, tx: &Transaction) -> Result<(), PoolError> {
		let mut txs = self.transactions
			.iter()
			.map(|x| *x.clone())
			.collect::<Vec<_>>();
		txs.push(tx.clone());
		let agg_tx =
			transaction::aggregate_with_cut_through(txs).map_err(|_| PoolError::GenericPoolError)?;

		// Validate aggregated tx against the chain txhashset extension.
		self.blockchain.validate_raw_tx(&agg_tx)?;

		// If we get here successfully then we can safely add the tx to the pool.
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
			"pool: reconcile_block: current pool txs - {}, candidate txs - {}",
			self.size(),
			candidate_transactions.len(),
		);

		// Clear the tx pool as we are about to start adding valid txs back in.
		self.clear();

		if candidate_transactions.is_empty() {
			debug!(LOGGER, "pool: reconcile_block: pool empty! Done.");
			assert!(self.is_empty());
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
		// Otherwise we need to expend effort adding (and validating)
		// them individually one-by-one.

		// Naive but robust - add everything back sequentially,
		// validate the pool each time.
		for tx in candidate_transactions {
			self.add_to_pool(&*tx);
		}

		// TODO - need to return the evicted txs (not yet used anywhere though)
		Ok(vec![])
	}

	fn all_transactions(self) -> Vec<Box<Transaction>> {
		self.transactions
	}

	pub fn size(&self) -> usize {
		self.transactions.len()
	}

	fn clear(&mut self) {
		self.transactions.clear()
	}

	fn is_empty(&self) -> bool {
		self.transactions.is_empty()
	}
}
