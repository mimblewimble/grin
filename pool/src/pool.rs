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

//! Transaction pool implementation.
//! Used for both the txpool and stempool layers in the pool.

use self::core::core::hash::{Hash, Hashed};
use self::core::core::id::{ShortId, ShortIdentifiable};
use self::core::core::transaction;
use self::core::core::verifier_cache::VerifierCache;
use self::core::core::{Block, BlockHeader, BlockSums, Committed, Transaction, TxKernel};
use self::util::RwLock;
use crate::types::{BlockChain, PoolEntry, PoolEntryState, PoolError};
use grin_core as core;
use grin_util as util;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct Pool {
	/// Entries in the pool (tx + info + timer) in simple insertion order.
	pub entries: Vec<PoolEntry>,
	/// The blockchain
	pub blockchain: Arc<dyn BlockChain>,
	pub verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	pub name: String,
}

impl Pool {
	pub fn new(
		chain: Arc<dyn BlockChain>,
		verifier_cache: Arc<RwLock<dyn VerifierCache>>,
		name: String,
	) -> Pool {
		Pool {
			entries: vec![],
			blockchain: chain,
			verifier_cache,
			name,
		}
	}

	/// Does the transaction pool contain an entry for the given transaction?
	pub fn contains_tx(&self, hash: Hash) -> bool {
		self.entries.iter().any(|x| x.tx.hash() == hash)
	}

	pub fn get_tx(&self, hash: Hash) -> Option<Transaction> {
		self.entries
			.iter()
			.find(|x| x.tx.hash() == hash)
			.map(|x| x.tx.clone())
	}

	/// Query the tx pool for an individual tx matching the given kernel hash.
	pub fn retrieve_tx_by_kernel_hash(&self, hash: Hash) -> Option<Transaction> {
		for x in &self.entries {
			for k in x.tx.kernels() {
				if k.hash() == hash {
					return Some(x.tx.clone());
				}
			}
		}
		None
	}

	/// Query the tx pool for all known txs based on kernel short_ids
	/// from the provided compact_block.
	/// Note: does not validate that we return the full set of required txs.
	/// The caller will need to validate that themselves.
	pub fn retrieve_transactions(
		&self,
		hash: Hash,
		nonce: u64,
		kern_ids: &[ShortId],
	) -> (Vec<Transaction>, Vec<ShortId>) {
		let mut txs = vec![];
		let mut found_ids = vec![];

		// Rehash all entries in the pool using short_ids based on provided hash and nonce.
		'outer: for x in &self.entries {
			for k in x.tx.kernels() {
				// rehash each kernel to calculate the block specific short_id
				let short_id = k.short_id(&hash, nonce);
				if kern_ids.contains(&short_id) {
					txs.push(x.tx.clone());
					found_ids.push(short_id);
				}
				if found_ids.len() == kern_ids.len() {
					break 'outer;
				}
			}
		}
		txs.dedup();
		(
			txs,
			kern_ids
				.into_iter()
				.filter(|id| !found_ids.contains(id))
				.cloned()
				.collect(),
		)
	}

	/// Take pool transactions, filtering and ordering them in a way that's
	/// appropriate to put in a mined block. Aggregates chains of dependent
	/// transactions, orders by fee over weight and ensures to total weight
	/// doesn't exceed block limits.
	pub fn prepare_mineable_transactions(
		&self,
		max_weight: usize,
	) -> Result<Vec<Transaction>, PoolError> {
		let header = self.blockchain.chain_head()?;
		let tx_buckets = self.bucket_transactions();

		// flatten buckets using aggregate (with cut-through)
		let mut flat_txs: Vec<Transaction> = tx_buckets
			.into_iter()
			.filter_map(|bucket| transaction::aggregate(bucket).ok())
			.filter(|x| x.validate(self.verifier_cache.clone()).is_ok())
			.collect();

		// sort by fees over weight, multiplying by 1000 to keep some precision
		// don't think we'll ever see a >max_u64/1000 fee transaction
		flat_txs.sort_unstable_by_key(|tx| tx.fee() * 1000 / tx.tx_weight() as u64);

		// accumulate as long as we're not above the block weight
		let mut weight = 0;
		flat_txs.retain(|tx| {
			weight += tx.tx_weight_as_block() as usize;
			weight < max_weight
		});

		// Iteratively apply the txs to the current chain state,
		// rejecting any that do not result in a valid state.
		// Return a vec of all the valid txs.
		let txs = self.validate_raw_txs(flat_txs, None, &header)?;
		Ok(txs)
	}

	pub fn all_transactions(&self) -> Vec<Transaction> {
		self.entries.iter().map(|x| x.tx.clone()).collect()
	}

	pub fn aggregate_transaction(&self) -> Result<Option<Transaction>, PoolError> {
		let txs = self.all_transactions();
		if txs.is_empty() {
			return Ok(None);
		}

		let tx = transaction::aggregate(txs)?;
		tx.validate(self.verifier_cache.clone())?;
		Ok(Some(tx))
	}

	pub fn select_valid_transactions(
		&self,
		txs: Vec<Transaction>,
		extra_tx: Option<Transaction>,
		header: &BlockHeader,
	) -> Result<Vec<Transaction>, PoolError> {
		let valid_txs = self.validate_raw_txs(txs, extra_tx, header)?;
		Ok(valid_txs)
	}

	pub fn get_transactions_in_state(&self, state: PoolEntryState) -> Vec<Transaction> {
		self.entries
			.iter()
			.filter(|x| x.state == state)
			.map(|x| x.tx.clone())
			.collect::<Vec<_>>()
	}

	// Transition the specified pool entries to the new state.
	pub fn transition_to_state(&mut self, txs: &[Transaction], state: PoolEntryState) {
		for x in &mut self.entries {
			if txs.contains(&x.tx) {
				x.state = state;
			}
		}
	}

	// Aggregate this new tx with all existing txs in the pool.
	// If we can validate the aggregated tx against the current chain state
	// then we can safely add the tx to the pool.
	pub fn add_to_pool(
		&mut self,
		entry: PoolEntry,
		extra_txs: Vec<Transaction>,
		header: &BlockHeader,
	) -> Result<(), PoolError> {
		// Combine all the txs from the pool with any extra txs provided.
		let mut txs = self.all_transactions();

		// Quick check to see if we have seen this tx before.
		if txs.contains(&entry.tx) {
			return Err(PoolError::DuplicateTx);
		}

		txs.extend(extra_txs);

		let agg_tx = if txs.is_empty() {
			// If we have nothing to aggregate then simply return the tx itself.
			entry.tx.clone()
		} else {
			// Create a single aggregated tx from the existing pool txs and the
			// new entry
			txs.push(entry.tx.clone());

			let tx = transaction::aggregate(txs)?;
			tx.validate(self.verifier_cache.clone())?;
			tx
		};

		// Validate aggregated tx against a known chain state.
		self.validate_raw_tx(&agg_tx, header)?;

		debug!(
			"add_to_pool [{}]: {} ({}) [in/out/kern: {}/{}/{}] pool: {} (at block {})",
			self.name,
			entry.tx.hash(),
			entry.src.debug_name,
			entry.tx.inputs().len(),
			entry.tx.outputs().len(),
			entry.tx.kernels().len(),
			self.size(),
			header.hash(),
		);
		// If we get here successfully then we can safely add the entry to the pool.
		self.entries.push(entry);

		Ok(())
	}

	fn validate_raw_tx(
		&self,
		tx: &Transaction,
		header: &BlockHeader,
	) -> Result<BlockSums, PoolError> {
		tx.validate(self.verifier_cache.clone())?;

		// Validate the tx against current chain state.
		// Check all inputs are in the current UTXO set.
		// Check all outputs are unique in current UTXO set.
		self.blockchain.validate_tx(tx)?;

		let new_sums = self.apply_tx_to_block_sums(tx, header)?;
		Ok(new_sums)
	}

	fn validate_raw_txs(
		&self,
		txs: Vec<Transaction>,
		extra_tx: Option<Transaction>,
		header: &BlockHeader,
	) -> Result<Vec<Transaction>, PoolError> {
		let mut valid_txs = vec![];

		for tx in txs {
			let mut candidate_txs = vec![];
			if let Some(extra_tx) = extra_tx.clone() {
				candidate_txs.push(extra_tx);
			};
			candidate_txs.extend(valid_txs.clone());
			candidate_txs.push(tx.clone());

			// Build a single aggregate tx from candidate txs.
			let agg_tx = transaction::aggregate(candidate_txs)?;

			// We know the tx is valid if the entire aggregate tx is valid.
			if self.validate_raw_tx(&agg_tx, header).is_ok() {
				valid_txs.push(tx);
			}
		}

		Ok(valid_txs)
	}

	fn apply_tx_to_block_sums(
		&self,
		tx: &Transaction,
		header: &BlockHeader,
	) -> Result<BlockSums, PoolError> {
		let overage = tx.overage();
		let offset = (header.total_kernel_offset() + tx.offset)?;

		let block_sums = self.blockchain.get_block_sums(&header.hash())?;

		// Verify the kernel sums for the block_sums with the new tx applied,
		// accounting for overage and offset.
		let (utxo_sum, kernel_sum) =
			(block_sums, tx as &dyn Committed).verify_kernel_sums(overage, offset)?;

		Ok(BlockSums {
			utxo_sum,
			kernel_sum,
		})
	}

	pub fn reconcile(
		&mut self,
		extra_tx: Option<Transaction>,
		header: &BlockHeader,
	) -> Result<(), PoolError> {
		let existing_entries = self.entries.clone();
		self.entries.clear();

		let mut extra_txs = vec![];
		if let Some(extra_tx) = extra_tx {
			extra_txs.push(extra_tx);
		}

		for x in existing_entries {
			let _ = self.add_to_pool(x, extra_txs.clone(), header);
		}

		Ok(())
	}

	// Group dependent transactions in buckets (vectors), each bucket
	// is therefore independent from the others. Relies on the entries
	// Vec having parent transactions first (should always be the case)
	fn bucket_transactions(&self) -> Vec<Vec<Transaction>> {
		let mut tx_buckets = vec![];
		let mut output_commits = HashMap::new();

		for entry in &self.entries {
			// check the commits index to find parents and their position
			// picking the last one for bucket (so all parents come first)
			let mut insert_pos: i32 = -1;
			for input in entry.tx.inputs() {
				if let Some(pos) = output_commits.get(&input.commitment()) {
					if *pos > insert_pos {
						insert_pos = *pos;
					}
				}
			}
			if insert_pos == -1 {
				// no parent, just add to the end in its own bucket
				insert_pos = tx_buckets.len() as i32;
				tx_buckets.push(vec![entry.tx.clone()]);
			} else {
				// parent found, add to its bucket
				tx_buckets[insert_pos as usize].push(entry.tx.clone());
			}

			// update the commits index
			for out in entry.tx.outputs() {
				output_commits.insert(out.commitment(), insert_pos);
			}
		}
		tx_buckets
	}

	pub fn find_matching_transactions(&self, kernels: &[TxKernel]) -> Vec<Transaction> {
		// While the inputs outputs can be cut-through the kernel will stay intact
		// In order to deaggregate tx we look for tx with the same kernel
		let mut found_txs = vec![];

		// Gather all the kernels of the multi-kernel transaction in one set
		let kernel_set = kernels.into_iter().collect::<HashSet<_>>();

		// Check each transaction in the pool
		for entry in &self.entries {
			let entry_kernel_set = entry.tx.kernels().iter().collect::<HashSet<_>>();
			if entry_kernel_set.is_subset(&kernel_set) {
				found_txs.push(entry.tx.clone());
			}
		}
		found_txs
	}

	/// Quick reconciliation step - we can evict any txs in the pool where
	/// inputs or kernels intersect with the block.
	pub fn reconcile_block(&mut self, block: &Block) {
		// Filter txs in the pool based on the latest block.
		// Reject any txs where we see a matching tx kernel in the block.
		// Also reject any txs where we see a conflicting tx,
		// where an input is spent in a different tx.
		self.entries.retain(|x| {
			!x.tx.kernels().iter().any(|y| block.kernels().contains(y))
				&& !x.tx.inputs().iter().any(|y| block.inputs().contains(y))
		});
	}

	pub fn size(&self) -> usize {
		self.entries.len()
	}
}
