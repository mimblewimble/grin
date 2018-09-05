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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use core::consensus;
use core::core::hash::{Hash, Hashed};
use core::core::id::ShortIdentifiable;
use core::core::transaction;
use core::core::verifier_cache::VerifierCache;
use core::core::{Block, CompactBlock, Transaction, TxKernel};
use types::{BlockChain, PoolEntry, PoolEntryState, PoolError};
use util::LOGGER;

// max weight leaving minimum space for a coinbase
const MAX_MINEABLE_WEIGHT: usize =
	consensus::MAX_BLOCK_WEIGHT - consensus::BLOCK_OUTPUT_WEIGHT - consensus::BLOCK_KERNEL_WEIGHT;

// longest chain of dependent transactions that can be included in a block
const MAX_TX_CHAIN: usize = 20;

pub struct Pool {
	/// Entries in the pool (tx + info + timer) in simple insertion order.
	pub entries: Vec<PoolEntry>,
	/// The blockchain
	pub blockchain: Arc<BlockChain>,
	pub verifier_cache: Arc<RwLock<VerifierCache>>,
	pub name: String,
}

impl Pool {
	pub fn new(
		chain: Arc<BlockChain>,
		verifier_cache: Arc<RwLock<VerifierCache>>,
		name: String,
	) -> Pool {
		Pool {
			entries: vec![],
			blockchain: chain.clone(),
			verifier_cache: verifier_cache.clone(),
			name,
		}
	}

	/// Does the transaction pool contain an entry for the given transaction?
	pub fn contains_tx(&self, tx: &Transaction) -> bool {
		self.entries.iter().any(|x| x.tx.hash() == tx.hash())
	}

	/// Query the tx pool for all known txs based on kernel short_ids
	/// from the provided compact_block.
	/// Note: does not validate that we return the full set of required txs.
	/// The caller will need to validate that themselves.
	pub fn retrieve_transactions(&self, cb: &CompactBlock) -> Vec<Transaction> {
		let mut txs = vec![];

		for x in &self.entries {
			for kernel in x.tx.kernels() {
				// rehash each kernel to calculate the block specific short_id
				let short_id = kernel.short_id(&cb.hash(), cb.nonce);

				// if any kernel matches then keep the tx for later
				if cb.kern_ids().contains(&short_id) {
					txs.push(x.tx.clone());
					break;
				}
			}
		}
		txs
	}

	/// Take pool transactions, filtering and ordering them in a way that's
	/// appropriate to put in a mined block. Aggregates chains of dependent
	/// transactions, orders by fee over weight and ensures to total weight
	/// doesn't exceed block limits.
	pub fn prepare_mineable_transactions(&self) -> Vec<Transaction> {
		let header = self.blockchain.chain_head().unwrap();

		let tx_buckets = self.bucket_transactions();

		// flatten buckets using aggregate (with cut-through)
		let mut flat_txs: Vec<Transaction> = tx_buckets
			.into_iter()
			.filter_map(|mut bucket| {
				bucket.truncate(MAX_TX_CHAIN);
				transaction::aggregate(bucket, self.verifier_cache.clone()).ok()
			})
			.collect();

		// sort by fees over weight, multiplying by 1000 to keep some precision
		// don't think we'll ever see a >max_u64/1000 fee transaction
		flat_txs.sort_unstable_by_key(|tx| tx.fee() * 1000 / tx.tx_weight() as u64);

		// accumulate as long as we're not above the block weight
		let mut weight = 0;
		flat_txs.retain(|tx| {
			weight += tx.tx_weight_as_block() as usize;
			weight < MAX_MINEABLE_WEIGHT
		});

		// make sure those txs are all valid together, no Error is expected
		// when passing None
		self.blockchain
			.validate_raw_txs(flat_txs, None, &header.hash())
			.expect("should never happen")
	}

	pub fn all_transactions(&self) -> Vec<Transaction> {
		self.entries.iter().map(|x| x.tx.clone()).collect()
	}

	pub fn aggregate_transaction(&self) -> Result<Option<Transaction>, PoolError> {
		let txs = self.all_transactions();
		if txs.is_empty() {
			return Ok(None);
		}

		let tx = transaction::aggregate(txs, self.verifier_cache.clone())?;
		Ok(Some(tx))
	}

	pub fn select_valid_transactions(
		&mut self,
		from_state: PoolEntryState,
		to_state: PoolEntryState,
		extra_tx: Option<Transaction>,
		block_hash: &Hash,
	) -> Result<Vec<Transaction>, PoolError> {
		let entries = &mut self
			.entries
			.iter_mut()
			.filter(|x| x.state == from_state)
			.collect::<Vec<_>>();

		let candidate_txs: Vec<Transaction> = entries.iter().map(|x| x.tx.clone()).collect();
		if candidate_txs.is_empty() {
			return Ok(vec![]);
		}
		let valid_txs = self
			.blockchain
			.validate_raw_txs(candidate_txs, extra_tx, block_hash)?;

		// Update state on all entries included in final vec of valid txs.
		for x in &mut entries.iter_mut() {
			if valid_txs.contains(&x.tx) {
				x.state = to_state.clone();
			}
		}

		Ok(valid_txs)
	}

	// Aggregate this new tx with all existing txs in the pool.
	// If we can validate the aggregated tx against the current chain state
	// then we can safely add the tx to the pool.
	pub fn add_to_pool(
		&mut self,
		entry: PoolEntry,
		extra_txs: Vec<Transaction>,
		block_hash: &Hash,
	) -> Result<(), PoolError> {
		debug!(
			LOGGER,
			"pool [{}]: add_to_pool: {}, {:?}, inputs: {}, outputs: {}, kernels: {} (at block {})",
			self.name,
			entry.tx.hash(),
			entry.src,
			entry.tx.inputs().len(),
			entry.tx.outputs().len(),
			entry.tx.kernels().len(),
			block_hash,
		);

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
			transaction::aggregate(txs, self.verifier_cache.clone())?
		};

		// Validate aggregated tx against a known chain state (via txhashset
		// extension).
		self.blockchain
			.validate_raw_txs(vec![], Some(agg_tx), block_hash)?;

		// If we get here successfully then we can safely add the entry to the pool.
		self.entries.push(entry);

		Ok(())
	}

	pub fn reconcile(
		&mut self,
		extra_tx: Option<Transaction>,
		block_hash: &Hash,
	) -> Result<(), PoolError> {
		let candidate_txs = self.all_transactions();
		let existing_len = candidate_txs.len();

		if candidate_txs.is_empty() {
			return Ok(());
		}

		// Go through the candidate txs and keep everything that validates incrementally
		// against a known chain state, accounting for the "extra tx" as necessary.
		let valid_txs = self
			.blockchain
			.validate_raw_txs(candidate_txs, extra_tx, block_hash)?;
		self.entries.retain(|x| valid_txs.contains(&x.tx));

		debug!(
			LOGGER,
			"pool [{}]: reconcile: existing txs {}, retained txs {}",
			self.name,
			existing_len,
			self.entries.len(),
		);

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

	// Filter txs in the pool based on the latest block.
	// Reject any txs where we see a matching tx kernel in the block.
	// Also reject any txs where we see a conflicting tx,
	// where an input is spent in a different tx.
	fn remaining_transactions(&self, block: &Block) -> Vec<Transaction> {
		self.entries
			.iter()
			.filter(|x| !x.tx.kernels().iter().any(|y| block.kernels().contains(y)))
			.filter(|x| !x.tx.inputs().iter().any(|y| block.inputs().contains(y)))
			.map(|x| x.tx.clone())
			.collect()
	}

	pub fn find_matching_transactions(&self, kernels: Vec<TxKernel>) -> Vec<Transaction> {
		// While the inputs outputs can be cut-through the kernel will stay intact
		// In order to deaggregate tx we look for tx with the same kernel
		let mut found_txs = vec![];

		// Gather all the kernels of the multi-kernel transaction in one set
		let kernel_set = kernels.into_iter().collect::<HashSet<_>>();

		// Check each transaction in the pool
		for entry in &self.entries {
			let entry_kernel_set = entry.tx.kernels().iter().cloned().collect::<HashSet<_>>();
			if entry_kernel_set.is_subset(&kernel_set) {
				found_txs.push(entry.tx.clone());
			}
		}
		found_txs
	}

	/// Quick reconciliation step - we can evict any txs in the pool where
	/// inputs or kernels intersect with the block.
	pub fn reconcile_block(&mut self, block: &Block) -> Result<(), PoolError> {
		let candidate_txs = self.remaining_transactions(block);
		self.entries.retain(|x| candidate_txs.contains(&x.tx));
		Ok(())
	}

	pub fn size(&self) -> usize {
		self.entries.len()
	}
}
