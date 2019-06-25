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
use self::core::core::{
	Block, BlockHeader, BlockSums, Committed, Transaction, TxKernel, Weighting,
};
use self::util::RwLock;
use crate::types::{BlockChain, PoolEntry, PoolError};
use grin_core as core;
use grin_util as util;
use std::cmp::Reverse;
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
	/// transactions, orders by fee over weight and ensures the total weight
	/// does not exceed the provided max_weight (miner defined block weight).
	pub fn prepare_mineable_transactions(
		&self,
		max_weight: usize,
	) -> Result<Vec<Transaction>, PoolError> {
		let weighting = Weighting::AsLimitedTransaction(max_weight);

		// Sort the txs in the pool via the "bucket" logic to -
		//   * maintain dependency ordering
		//   * maximize cut-through
		//   * maximize overall fees
		let txs = self.bucket_transactions(weighting);

		// Iteratively apply the txs to the current chain state,
		// rejecting any that do not result in a valid state.
		// Verify these txs produce an aggregated tx below max_weight.
		// Return a vec of all the valid txs.
		let header = self.blockchain.chain_head()?;
		let valid_txs = self.validate_raw_txs(&txs, None, &header, weighting)?;
		Ok(valid_txs)
	}

	pub fn all_transactions(&self) -> Vec<Transaction> {
		self.entries.iter().map(|x| x.tx.clone()).collect()
	}

	/// Return a single aggregate tx representing all txs in the txpool.
	/// Returns None if the txpool is empty.
	pub fn all_transactions_aggregate(&self) -> Result<Option<Transaction>, PoolError> {
		let txs = self.all_transactions();
		if txs.is_empty() {
			return Ok(None);
		}

		let tx = transaction::aggregate(txs)?;

		// Validate the single aggregate transaction "as pool", not subject to tx weight limits.
		tx.validate(Weighting::NoLimit, self.verifier_cache.clone())?;

		Ok(Some(tx))
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
			transaction::aggregate(txs)?
		};

		// Validate aggregated tx (existing pool + new tx), ignoring tx weight limits.
		// Validate against known chain state at the provided header.
		self.validate_raw_tx(&agg_tx, header, Weighting::NoLimit)?;

		// If we get here successfully then we can safely add the entry to the pool.
		self.log_pool_add(&entry, header);
		self.entries.push(entry);

		Ok(())
	}

	fn log_pool_add(&self, entry: &PoolEntry, header: &BlockHeader) {
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
	}

	fn validate_raw_tx(
		&self,
		tx: &Transaction,
		header: &BlockHeader,
		weighting: Weighting,
	) -> Result<BlockSums, PoolError> {
		// Validate the tx, conditionally checking against weight limits,
		// based on weight verification type.
		tx.validate(weighting, self.verifier_cache.clone())?;

		// Validate the tx against current chain state.
		// Check all inputs are in the current UTXO set.
		// Check all outputs are unique in current UTXO set.
		self.blockchain.validate_tx(tx)?;

		let new_sums = self.apply_tx_to_block_sums(tx, header)?;
		Ok(new_sums)
	}

	pub fn validate_raw_txs(
		&self,
		txs: &[Transaction],
		extra_tx: Option<Transaction>,
		header: &BlockHeader,
		weighting: Weighting,
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
			if self.validate_raw_tx(&agg_tx, header, weighting).is_ok() {
				valid_txs.push(tx.clone());
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
		let offset = (header.total_kernel_offset() + tx.offset.clone())?;

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

	/// Buckets consist of a vec of txs and track the aggregate fee_to_weight.
	/// We aggregate (cut-through) dependent transactions within a bucket *unless* adding a tx
	/// would reduce the aggregate fee_to_weight, in which case we start a new bucket.
	/// Note this new bucket will by definition have a lower fee_to_weight than the bucket
	/// containing the tx it depends on.
	/// Sorting the buckets by fee_to_weight will therefore preserve dependency ordering,
	/// maximizing both cut-through and overall fees.
	pub fn bucket_transactions(&self, weighting: Weighting) -> Vec<Transaction> {
		let mut tx_buckets: Vec<Bucket> = Vec::new();
		let mut output_commits = HashMap::new();
		let mut rejected = HashSet::new();

		for entry in &self.entries {
			// check the commits index to find parents and their position
			// if single parent then we are good, we can bucket it with its parent
			// if multiple parents then we need to combine buckets, but for now simply reject it (rare case)
			let mut insert_pos = None;
			let mut is_rejected = false;

			for input in entry.tx.inputs() {
				if rejected.contains(&input.commitment()) {
					// Depends on a rejected tx, so reject this one.
					is_rejected = true;
					continue;
				} else if let Some(pos) = output_commits.get(&input.commitment()) {
					if insert_pos.is_some() {
						// Multiple dependencies so reject this tx (pick it up in next block).
						is_rejected = true;
						continue;
					} else {
						// Track the pos of the bucket we fall into.
						insert_pos = Some(*pos);
					}
				}
			}

			// If this tx is rejected then store all output commitments in our rejected set.
			if is_rejected {
				for out in entry.tx.outputs() {
					rejected.insert(out.commitment());
				}

				// Done with this entry (rejected), continue to next entry.
				continue;
			}

			match insert_pos {
				None => {
					// No parent tx, just add to the end in its own bucket.
					// This is the common case for non 0-conf txs in the txpool.
					// We assume the tx is valid here as we validated it on the way into the txpool.
					insert_pos = Some(tx_buckets.len());
					tx_buckets.push(Bucket::new(entry.tx.clone()));
				}
				Some(pos) => {
					// We found a single parent tx, so aggregate in the bucket
					// if the aggregate tx is a valid tx.
					// Otherwise discard and let the next block pick this tx up.
					let bucket = &tx_buckets[pos];

					if let Ok(new_bucket) = bucket.aggregate_with_tx(
						entry.tx.clone(),
						weighting,
						self.verifier_cache.clone(),
					) {
						if new_bucket.fee_to_weight >= bucket.fee_to_weight {
							// Only aggregate if it would not reduce the fee_to_weight ratio.
							tx_buckets[pos] = new_bucket;
						} else {
							// Otherwise put it in its own bucket at the end.
							// Note: This bucket will have a lower fee_to_weight
							// than the bucket it depends on.
							tx_buckets.push(Bucket::new(entry.tx.clone()));
						}
					} else {
						// Aggregation failed so discard this new tx.
						is_rejected = true;
					}
				}
			}

			if is_rejected {
				for out in entry.tx.outputs() {
					rejected.insert(out.commitment());
				}
			} else if let Some(insert_pos) = insert_pos {
				// We successfully added this tx to our set of buckets.
				// Update commits index for subsequent txs.
				for out in entry.tx.outputs() {
					output_commits.insert(out.commitment(), insert_pos);
				}
			}
		}

		// Sort them by fee_to_weight (descending).
		// Txs with no dependencies will be toward the start of the vec.
		// Txs with a big chain of dependencies will be toward the end of the vec.
		tx_buckets.sort_unstable_by_key(|x| Reverse(x.fee_to_weight));

		tx_buckets
			.into_iter()
			.map(|x| x.raw_txs)
			.flatten()
			.collect()
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

	/// Size of the pool.
	pub fn size(&self) -> usize {
		self.entries.len()
	}

	/// Is the pool empty?
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}
}

struct Bucket {
	raw_txs: Vec<Transaction>,
	fee_to_weight: u64,
}

impl Bucket {
	fn new(tx: Transaction) -> Bucket {
		Bucket {
			fee_to_weight: tx.fee_to_weight(),
			raw_txs: vec![tx.clone()],
		}
	}

	fn aggregate_with_tx(
		&self,
		new_tx: Transaction,
		weighting: Weighting,
		verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	) -> Result<Bucket, PoolError> {
		let mut raw_txs = self.raw_txs.clone();
		raw_txs.push(new_tx);
		let agg_tx = transaction::aggregate(raw_txs.clone())?;
		agg_tx.validate(weighting, verifier_cache)?;
		Ok(Bucket {
			fee_to_weight: agg_tx.fee_to_weight(),
			raw_txs: raw_txs,
		})
	}
}
