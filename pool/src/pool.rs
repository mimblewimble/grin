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
	/// Entries in the pool (tx + info + timer) in simple insertion order.
	pub entries: Vec<PoolEntry>,
	/// The blockchain
	pub blockchain: Arc<T>,
}

impl<T> Pool<T>
where
	T: BlockChain,
{
	pub fn new(chain: Arc<T>) -> Pool<T> {
		Pool {
			entries: vec![],
			blockchain: chain.clone(),
		}
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
			self.entries.len(),
		);

		let mut txs = vec![];

		for x in &self.entries {
			for kernel in &x.tx.kernels {
				// rehash each kernel to calculate the block specific short_id
				let short_id = kernel.short_id(&cb.hash(), cb.nonce);

				// if any kernel matches then keep the tx for later
				if cb.kern_ids.contains(&short_id) {
					txs.push(x.tx.clone());
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

	/// Take the first num_to_fetch txs based on insertion order.
	pub fn prepare_mineable_transactions(&self, num_to_fetch: u32) -> Vec<Transaction> {
		self.entries
			.iter()
			.take(num_to_fetch as usize)
			.map(|x| x.tx.clone())
			.collect()
	}

	pub fn all_transactions(&self) -> Vec<Transaction> {
		self.entries.iter().map(|x| x.tx.clone()).collect()
	}

	pub fn aggregate_transaction(&self) -> Result<Transaction, PoolError> {
		let txs = self.all_transactions();
		let tx = transaction::aggregate_with_cut_through(txs)?;
		Ok(tx)
	}

	// Aggregate this new tx with all existing txs in the pool.
	// If we can validate the aggregated tx against the current chain state
	// then we can safely add the tx to the pool.
	pub fn add_to_pool(
		&mut self,
		entry: PoolEntry,
		extra_txs: Vec<Transaction>,
	) -> Result<(), PoolError> {
		let mut txs = self.all_transactions();
		txs.push(entry.tx.clone());
		txs.extend(extra_txs);

		// Create a single aggregated tx from txs in the pool,
		// the tx in the proposed entry and any provided extra txs.
		let agg_tx = transaction::aggregate_with_cut_through(txs)?;

		// Validate aggregated tx against the chain txhashset extension.
		self.blockchain.validate_raw_tx(&agg_tx)?;

		// If we get here successfully then we can safely add the entry to the pool.
		self.entries.push(entry);

		Ok(())
	}

	// Simple check comparing tx kernels against kernels in the latest block.
	// This covers the trivial case where we just emptied the pool.
	fn remaining_transactions(&self, block: &Block) -> Vec<Transaction> {
		self.entries
			.iter()
			.filter(|x| !x.tx.kernels.iter().any(|k| block.kernels.contains(k)))
			.map(|x| x.tx.clone())
			.collect()
	}

	fn filtered_entries(&self, tx_hashes: Vec<Hash>) -> Vec<PoolEntry> {
		self.entries
			.iter()
			.filter(|x| tx_hashes.contains(&x.tx.hash()))
			.cloned()
			.collect()
	}

	// TODO - not fully implemented
	pub fn reconcile_block(
		&mut self,
		block: &Block,
		extra_tx: Option<&Transaction>,
	) -> Result<Vec<Transaction>, PoolError> {
		let candidate_txs = self.remaining_transactions(block);

		debug!(
			LOGGER,
			"pool: reconcile_block: current pool txs - {}, candidate txs - {}",
			self.size(),
			candidate_txs.len(),
		);

		if candidate_txs.is_empty() {
			self.clear();
			debug!(LOGGER, "pool: reconcile_block: pool empty! Done.");
			return Ok(vec![]);
		}

		// Go through the candidate txs and keep everything that validates incrementally
		// against the current chain state, accounting for the "extra tx" as necessary.

		let valid_txs = self.blockchain.validate_raw_txs(candidate_txs, extra_tx)?;
		self.entries.retain(|x| valid_txs.contains(&x.tx));

		// TODO - need to return the evicted txs (not yet used anywhere though)
		Ok(vec![])
	}

	pub fn size(&self) -> usize {
		self.entries.len()
	}

	fn clear(&mut self) {
		self.entries.clear();
	}
}
