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

//! Top-level Pool type, methods, and tests

use rand;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use time;

use core::core::hash::Hash;
use core::core::hash::Hashed;
use core::core::id::ShortIdentifiable;
use core::core::transaction;
use core::core::{OutputIdentifier, Transaction, TxKernel};
use core::core::{block, hash};
use util::LOGGER;
use util::secp::pedersen::Commitment;

pub use graph;
use types::*;

/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
pub struct TransactionPool<T> {
	/// configuration
	pub config: PoolConfig,
	/// All transactions hash in the stempool with a time attached to ensure
	/// propagation
	pub time_stem_transactions: HashMap<hash::Hash, i64>,
	/// All transactions in the stempool
	pub stem_transactions: HashMap<hash::Hash, transaction::Transaction>,
	/// All transactions in the pool
	pub transactions: HashMap<hash::Hash, transaction::Transaction>,
	/// The stem pool
	pub stempool: Pool,
	/// The pool itself
	pub pool: Pool,
	/// Orphans in the pool
	pub orphans: Orphans,
	/// blockchain is a DummyChain, for now, which mimics what the future
	/// chain will offer to the pool
	pub blockchain: Arc<T>,
	/// Adapter
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
			time_stem_transactions: HashMap::new(),
			stem_transactions: HashMap::new(),
			transactions: HashMap::new(),
			stempool: Pool::empty(),
			pool: Pool::empty(),
			orphans: Orphans::empty(),
			blockchain: chain,
			adapter: adapter,
		}
	}

	/// Query the tx pool for all known txs based on kernel short_ids
	/// from the provided compact_block.
	/// Note: does not validate that we return the full set of required txs.
	/// The caller will need to validate that themselves.
	pub fn retrieve_transactions(&self, cb: &block::CompactBlock) -> Vec<Transaction> {
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

	/// Searches for an output, designated by its commitment, from the current
	/// best Output view, presented by taking the best blockchain Output set (as
	/// determined by the blockchain component) and rectifying pool spent and
	/// unspents.
	/// Detects double spends and unknown references from the pool and
	/// blockchain only; any conflicts with entries in the orphans set must
	/// be accounted for separately, if relevant.
	pub fn search_for_best_output(&self, output_ref: &OutputIdentifier) -> Parent {
		// The current best unspent set is:
		//   Pool unspent + (blockchain unspent - pool->blockchain spent)
		//   Pool unspents are unconditional so we check those first
		self.search_stempool_spents(&output_ref.commit)
			.or(self.pool.get_available_output(&output_ref.commit).map(|x| {
				let tx_ref = x.source_hash().unwrap();
				Parent::PoolTransaction { tx_ref }
			}))
			.or(self.stempool
				.get_available_output(&output_ref.commit)
				.map(|x| {
					let tx_ref = x.source_hash().unwrap();
					Parent::StemPoolTransaction { tx_ref }
				}))
			.or(self.search_blockchain_unspents(output_ref))
			.or(self.search_pool_spents(&output_ref.commit))
			.unwrap_or(Parent::Unknown)
	}

	// search_blockchain_unspents searches the current view of the blockchain
	// unspent set, represented by blockchain unspents - pool spents, for an
	// output designated by output_commitment.
	fn search_blockchain_unspents(&self, output_ref: &OutputIdentifier) -> Option<Parent> {
		self.blockchain.is_unspent(output_ref).ok().map(|_| {
			match self.pool.get_blockchain_spent(&output_ref.commit) {
				Some(x) => {
					let other_tx = x.destination_hash().unwrap();
					Parent::AlreadySpent { other_tx }
				}
				None => match self.stempool.get_blockchain_spent(&output_ref.commit) {
					Some(x) => {
						let other_tx = x.destination_hash().unwrap();
						Parent::AlreadySpent { other_tx }
					}
					None => Parent::BlockTransaction,
				},
			}
		})
	}

	// search_pool_spents is the second half of pool input detection, after the
	// available_outputs have been checked. This returns either a
	// Parent::AlreadySpent or None.
	fn search_pool_spents(&self, output_commitment: &Commitment) -> Option<Parent> {
		self.pool
			.get_internal_spent(output_commitment)
			.map(|x| Parent::AlreadySpent {
				other_tx: x.destination_hash().unwrap(),
			})
	}

	// search_pool_spents is the second half of pool input detection, after the
	// available_outputs have been checked. This returns either a
	// Parent::AlreadySpent or None.
	fn search_stempool_spents(&self, output_commitment: &Commitment) -> Option<Parent> {
		self.stempool
			.get_internal_spent(output_commitment)
			.map(|x| Parent::AlreadySpent {
				other_tx: x.destination_hash().unwrap(),
			})
	}

	/// Get the number of transactions in the stempool
	pub fn stempool_size(&self) -> usize {
		self.stempool.num_transactions()
	}

	/// Get the number of transactions in the pool
	pub fn pool_size(&self) -> usize {
		self.pool.num_transactions()
	}

	/// Get the number of orphans in the pool
	pub fn orphans_size(&self) -> usize {
		self.orphans.num_transactions()
	}

	/// Get the total size (stem transactions + transactions + orphans) of the
	/// pool
	pub fn total_size(&self) -> usize {
		self.stempool.num_transactions() + self.pool.num_transactions()
			+ self.orphans.num_transactions()
	}

	/// Attempts to add a transaction to the stempool or the memory pool.
	///
	/// Adds a transaction to the stem memory pool, deferring to the orphans
	/// pool if necessary, and performing any connection-related validity
	/// checks. Happens under an exclusive mutable reference gated by the
	/// write portion of a RWLock.
	pub fn add_to_memory_pool(
		&mut self,
		_: TxSource,
		tx: transaction::Transaction,
		stem: bool,
	) -> Result<(), PoolError> {
		// Do we have the capacity to accept this transaction?
		if let Err(e) = self.is_acceptable(&tx) {
			return Err(e);
		}

		// Making sure the transaction is valid before anything else.
		tx.validate().map_err(|e| PoolError::InvalidTx(e))?;

		// The first check involves ensuring that an indentical transaction is not
		// alreay in the stem transaction or regular transaction pool.
		// A non-authoritative similar check should be performed under the
		// pool's read lock before we get to this point, which would catch the
		// majority of duplicate cases. The race condition is caught here.
		// TODO: When the transaction identifier is finalized, the assumptions
		// here may change depending on the exact coverage of the identifier.
		// The current tx.hash() method, for example, does not cover changes
		// to fees or other elements of the signature preimage.
		let tx_hash = graph::transaction_identifier(&tx);
		if let Err(e) = self.check_pools(&tx_hash, stem) {
			return Err(e);
		}

		// Check that the transaction is mature
		let head_header = self.blockchain.head_header()?;
		if let Err(e) = self.is_mature(&tx, &head_header) {
			return Err(e);
		}

		// Here if we have a stem transaction, decide wether it will be broadcasted
		// in stem or fluff phase
		let mut rng = rand::thread_rng();
		let random = rng.gen_range(0, 101);
		let stem_propagation = random <= self.config.dandelion_probability;
		let mut will_stem = stem && stem_propagation;

		// Track the case where a parent of a transaction is in stempool
		let mut parent_in_stempool = false;
		// The timer attached to this transaction
		let mut timer: i64 = 0;

		// The next issue is to identify all unspent outputs that
		// this transaction will consume and make sure they exist in the set.
		let mut pool_refs: Vec<graph::Edge> = Vec::new();
		let mut orphan_refs: Vec<graph::Edge> = Vec::new();
		let mut blockchain_refs: Vec<graph::Edge> = Vec::new();

		for input in &tx.inputs {
			let output = OutputIdentifier::from_input(&input);
			let base = graph::Edge::new(None, Some(tx_hash), output.clone());

			// Note that search_for_best_output does not examine orphans, by
			// design. If an incoming transaction consumes pool outputs already
			// spent by the orphans set, this does not preclude its inclusion
			// into the pool.
			match self.search_for_best_output(&output) {
				Parent::PoolTransaction { tx_ref: x } => pool_refs.push(base.with_source(Some(x))),
				Parent::StemPoolTransaction { tx_ref: x } => {
					will_stem = true;
					parent_in_stempool = true;
					debug!(LOGGER, "Parent is in stempool, going in stempool");
					pool_refs.push(base.with_source(Some(x)));
					let temp_timer = self.time_stem_transactions.get(&x).unwrap().clone();
					if temp_timer > timer {
						timer = temp_timer;
					}
				}
				Parent::BlockTransaction => {
					let height = head_header.height + 1;
					self.blockchain.is_matured(&input, height)?;
					blockchain_refs.push(base);
				}
				Parent::Unknown => orphan_refs.push(base),
				Parent::AlreadySpent { other_tx: x } => {
					return Err(PoolError::DoubleSpend {
						other_tx: x,
						spent_output: input.commitment(),
					})
				}
			}
		}

		let is_orphan = orphan_refs.len() > 0;

		// In the case the parent is not in stempool we randomize the timer
		if !parent_in_stempool {
			timer = time::now_utc().to_timespec().sec + rand::thread_rng().gen_range(0, 31);
		}

		// Next we examine the outputs this transaction creates and ensure
		// that they do not already exist.
		// I believe its worth preventing duplicate outputs from being
		// accepted, even though it is possible for them to be mined
		// with strict ordering. In the future, if desirable, this could
		// be node policy config or more intelligent.
		for output in &tx.outputs {
			self.check_duplicate_outputs(output, is_orphan)?
		}

		// Assertion: we have exactly as many resolved spending references as
		// inputs to the transaction.
		assert_eq!(
			tx.inputs.len(),
			blockchain_refs.len() + pool_refs.len() + orphan_refs.len()
		);

		// At this point we know if we're spending all known unspents and not
		// creating any duplicate unspents.
		let pool_entry = graph::PoolEntry::new(&tx);
		let new_unspents = tx.outputs
			.iter()
			.map(|x| {
				let output = OutputIdentifier::from_output(&x);
				graph::Edge::new(Some(tx_hash), None, output)
			})
			.collect();

		if !is_orphan {
			// In the non-orphan (pool) case, we've ensured that every input
			// maps one-to-one with an unspent (available) output, and each
			// output is unique. No further checks are necessary.
			if will_stem {
				// Stem phase: transaction is added to the stem memory pool and broadcasted to a
				// randomly selected node.
				self.stempool.add_stempool_transaction(
					pool_entry,
					blockchain_refs,
					pool_refs,
					new_unspents,
				);

				self.adapter.stem_tx_accepted(&tx);
				self.stem_transactions.insert(tx_hash, tx);
				// Track this transaction
				self.time_stem_transactions.insert(tx_hash, timer);
			} else {
				// Fluff phase: transaction is added to memory pool and broadcasted normally
				self.pool.add_pool_transaction(
					pool_entry,
					blockchain_refs,
					pool_refs,
					new_unspents,
				);
				self.adapter.tx_accepted(&tx);
				self.transactions.insert(tx_hash, tx);
			}
			self.reconcile_orphans().unwrap();
			Ok(())
		} else {
			// At this point, we're pretty sure the transaction is an orphan,
			// but we have to explicitly check for double spends against the
			// orphans set; we do not check this as part of the connectivity
			// checking above.
			// First, any references resolved to the pool need to be compared
			// against active orphan pool_connections.
			// Note that pool_connections here also does double duty to
			// account for blockchain connections.
			for pool_ref in pool_refs.iter().chain(blockchain_refs.iter()) {
				match self.orphans
					.get_external_spent_output(&pool_ref.output_commitment())
				{
					// Should the below err be subtyped to orphans somehow?
					Some(x) => {
						return Err(PoolError::DoubleSpend {
							other_tx: x.destination_hash().unwrap(),
							spent_output: x.output_commitment(),
						})
					}
					None => {}
				}
			}

			// Next, we have to consider the possibility of double spends
			// within the orphans set.
			// We also have to distinguish now between missing and internal
			// references.
			let missing_refs = self.resolve_orphan_refs(tx_hash, &mut orphan_refs)?;

			// We have passed all failure modes.
			pool_refs.append(&mut blockchain_refs);
			error!(LOGGER, "Add to orphan");
			self.orphans.add_orphan_transaction(
				pool_entry,
				pool_refs,
				orphan_refs,
				missing_refs,
				new_unspents,
			);

			Err(PoolError::OrphanTransaction)
		}
	}

	/// Attempt to deaggregate a transaction and add it to the mempool
	pub fn deaggregate_and_add_to_memory_pool(
		&mut self,
		tx_source: TxSource,
		tx: transaction::Transaction,
		stem: bool,
	) -> Result<(), PoolError> {
		match self.deaggregate_transaction(tx.clone()) {
			Ok(deaggragated_tx) => self.add_to_memory_pool(tx_source, deaggragated_tx, stem),
			Err(e) => {
				debug!(
					LOGGER,
					"Could not deaggregate multi-kernel transaction: {:?}", e
				);
				self.add_to_memory_pool(tx_source, tx, stem)
			}
		}
	}

	/// Attempt to deaggregate multi-kernel transaction as much as possible
	/// based on the content of the mempool
	pub fn deaggregate_transaction(
		&self,
		tx: transaction::Transaction,
	) -> Result<Transaction, PoolError> {
		// find candidates tx and attempt to deaggregate
		match self.find_candidates(tx.clone()) {
			Some(candidates_txs) => match transaction::deaggregate(tx, candidates_txs) {
				Ok(deaggregated_tx) => Ok(deaggregated_tx),
				Err(e) => {
					debug!(LOGGER, "Could not deaggregate transaction: {}", e);
					Err(PoolError::FailedDeaggregation)
				}
			},
			None => {
				debug!(
					LOGGER,
					"Could not deaggregate transaction: no candidate transaction found"
				);
				Err(PoolError::FailedDeaggregation)
			}
		}
	}

	/// Find candidate transactions for a multi-kernel transaction
	fn find_candidates(&self, tx: transaction::Transaction) -> Option<Vec<Transaction>> {
		// While the inputs outputs can be cut-through the kernel will stay intact
		// In order to deaggregate tx we look for tx with the same kernel
		let mut found_txs: Vec<Transaction> = vec![];

		// Gather all the kernels of the multi-kernel transaction in one set
		let kernels_set: HashSet<TxKernel> = tx.kernels.iter().cloned().collect::<HashSet<_>>();

		// Check each transaction in the pool
		for (_, tx) in &self.transactions {
			let candidates_kernels_set: HashSet<TxKernel> =
				tx.kernels.iter().cloned().collect::<HashSet<_>>();

			let kernels_set_intersection: HashSet<&TxKernel> =
				kernels_set.intersection(&candidates_kernels_set).collect();

			// Consider the transaction only if all the kernels match and if it is indeed a
			// subset
			if kernels_set_intersection.len() == tx.kernels.len()
				&& candidates_kernels_set.is_subset(&kernels_set)
			{
				debug!(LOGGER, "Found a transaction with the same kernel");
				found_txs.push(tx.clone());
			}
		}

		if found_txs.len() != 0 {
			Some(found_txs)
		} else {
			None
		}
	}

	/// Check the output for a conflict with an existing output.
	///
	/// Checks the output (by commitment) against outputs in the blockchain
	/// or in the pool. If the transaction is destined for orphans, the
	/// orphans set is checked as well.
	fn check_duplicate_outputs(
		&self,
		output: &transaction::Output,
		is_orphan: bool,
	) -> Result<(), PoolError> {
		// Checking against current blockchain unspent outputs
		// We want outputs even if they're spent by pool txs, so we ignore
		// consumed_blockchain_outputs
		let out = OutputIdentifier::from_output(&output);
		if self.blockchain.is_unspent(&out).is_ok() {
			return Err(PoolError::DuplicateOutput {
				other_tx: None,
				in_chain: true,
				output: out.commit,
			});
		}

		// Check for existence of this output in the pool
		match self.pool.find_output(&output.commitment()) {
			Some(x) => {
				return Err(PoolError::DuplicateOutput {
					other_tx: Some(x),
					in_chain: false,
					output: output.commit,
				})
			}
			None => {}
		};

		// Check for existence of this output in the stempool
		match self.stempool.find_output(&output.commitment()) {
			Some(x) => {
				return Err(PoolError::DuplicateOutput {
					other_tx: Some(x),
					in_chain: false,
					output: output.commit,
				})
			}
			None => {}
		};

		// If the transaction might go into orphans, perform the same
		// checks as above but against the orphan set instead.
		if is_orphan {
			// Checking against orphan outputs
			match self.orphans.find_output(&output.commitment()) {
				Some(x) => {
					return Err(PoolError::DuplicateOutput {
						other_tx: Some(x),
						in_chain: false,
						output: output.commitment(),
					})
				}
				None => {}
			};
			// No need to check pool connections since those are covered
			// by pool unspents and blockchain connections.
		}
		Ok(())
	}

	/// Distinguish between missing, unspent, and spent orphan refs.
	///
	/// Takes the set of orphans_refs produced during transaction connectivity
	/// validation, which do not point at valid unspents in the blockchain or
	/// pool. These references point at either a missing (orphaned) commitment,
	/// an unspent output of the orphans set, or a spent output either within
	/// the orphans set or externally from orphans to the pool or blockchain.
	/// The last case results in a failure condition and transaction acceptance
	/// is aborted.
	fn resolve_orphan_refs(
		&self,
		tx_hash: hash::Hash,
		orphan_refs: &mut Vec<graph::Edge>,
	) -> Result<HashMap<usize, ()>, PoolError> {
		let mut missing_refs: HashMap<usize, ()> = HashMap::new();
		for (i, orphan_ref) in orphan_refs.iter_mut().enumerate() {
			let orphan_commitment = &orphan_ref.output_commitment();
			match self.orphans.get_available_output(&orphan_commitment) {
				// If the edge is an available output of orphans,
				// update the prepared edge
				Some(x) => *orphan_ref = x.with_destination(Some(tx_hash)),
				// If the edge is not an available output, it is either
				// already consumed or it belongs in missing_refs.
				None => {
					match self.orphans.get_internal_spent(&orphan_commitment) {
						Some(x) => {
							return Err(PoolError::DoubleSpend {
								other_tx: x.destination_hash().unwrap(),
								spent_output: x.output_commitment(),
							})
						}
						None => {
							// The reference does not resolve to anything.
							// Make sure this missing_output has not already
							// been claimed, then add this entry to
							// missing_refs
							match self.orphans.get_unknown_output(&orphan_commitment) {
								Some(x) => {
									return Err(PoolError::DoubleSpend {
										other_tx: x.destination_hash().unwrap(),
										spent_output: x.output_commitment(),
									})
								}
								None => missing_refs.insert(i, ()),
							};
						}
					};
				}
			};
		}
		Ok(missing_refs)
	}

	/// The primary goal of the reconcile_orphans method is to eliminate any
	/// orphans who conflict with the recently accepted pool transaction.
	/// TODO: How do we handle fishing orphans out that look like they could
	/// be freed? Current thought is to do so under a different lock domain
	/// so that we don't have the potential for long recursion under the write
	/// lock.
	pub fn reconcile_orphans(&self) -> Result<(), PoolError> {
		Ok(())
	}

	/// Updates the pool with the details of a new block.
	///
	/// Along with add_to_memory_pool, reconcile_block is the other major entry
	/// point for the transaction pool. This method reconciles the records in
	/// the transaction pool with the updated view presented by the incoming
	/// block. This involves removing any transactions which appear to conflict
	/// with inputs and outputs consumed in the block, and invalidating any
	/// descendents or parents of the removed transaction, where relevant.
	///
	/// Returns a list of transactions which have been evicted from the pool
	/// due to the recent block. Because transaction association information is
	/// irreversibly lost in the blockchain, we must keep track of these
	/// evicted transactions elsewhere so that we can make a best effort at
	/// returning them to the pool in the event of a reorg that invalidates
	/// this block.
	/// TODO also consider stempool here
	pub fn reconcile_block(
		&mut self,
		block: &block::Block,
	) -> Result<Vec<transaction::Transaction>, PoolError> {
		// If this pool has been kept in sync correctly, serializing all
		// updates, then the inputs must consume only members of the blockchain
		// output set.
		// If the block has been resolved properly and reduced fully to its
		// canonical form, no inputs may consume outputs generated by previous
		// transactions in the block; they would be cut-through. TODO: If this
		// is not consensus enforced, then logic must be added here to account
		// for that.
		// Based on this, we operate under the following algorithm:
		// For each block input, we examine the pool transaction, if any, that
		// consumes the same blockchain output.
		// If one exists, we mark the transaction and then examine its
		// children. Recursively, we mark each child until a child is
		// fully satisfied by outputs in the updated output view (after
		// reconciliation of the block), or there are no more children.
		//
		// Additionally, to protect our invariant dictating no duplicate
		// outputs, each output generated by the new output set is checked
		// against outputs generated by the pool and the corresponding
		// transactions are also marked.
		//
		// After marking concludes, sweeping begins. In order, the marked
		// transactions are removed, the vertexes corresponding to the
		// transactions are removed, all the marked transactions' outputs are
		// removed, and all remaining non-blockchain inputs are returned to the
		// unspent_outputs set.
		//
		// After the pool has been successfully processed, an orphans
		// reconciliation job is triggered.
		let mut marked_transactions: HashSet<hash::Hash> = HashSet::new();
		let mut marked_stem_transactions: HashSet<hash::Hash> = HashSet::new();

		{
			// find all conflicting txs based on inputs to the block
			let conflicting_txs: HashSet<hash::Hash> = block
				.inputs
				.iter()
				.filter_map(|x| self.pool.get_external_spent_output(&x.commitment()))
				.filter_map(|x| x.destination_hash())
				.collect();

			// find all conflicting stem txs based on inputs to the block
			let conflicting_stem_txs: HashSet<hash::Hash> = block
				.inputs
				.iter()
				.filter_map(|x| self.stempool.get_external_spent_output(&x.commitment()))
				.filter_map(|x| x.destination_hash())
				.collect();

			// find all outputs that conflict - potential for duplicates so use a HashSet
			// here
			let conflicting_outputs: HashSet<hash::Hash> = block
				.outputs
				.iter()
				.filter_map(|x: &transaction::Output| {
					self.pool
						.get_internal_spent_output(&x.commitment())
						.or(self.pool.get_available_output(&x.commitment()))
				})
				.filter_map(|x| x.source_hash())
				.collect();

			// Similarly find all outputs that conflict in the stempool- potential for
			// duplicates so use a HashSet here
			let conflicting_stem_outputs: HashSet<hash::Hash> = block
				.outputs
				.iter()
				.filter_map(|x: &transaction::Output| {
					self.stempool
						.get_internal_spent_output(&x.commitment())
						.or(self.stempool.get_available_output(&x.commitment()))
				})
				.filter_map(|x| x.source_hash())
				.collect();

			// now iterate over all conflicting hashes from both txs and outputs
			// we can just use the union of the two sets here to remove duplicates
			for &txh in conflicting_txs.union(&conflicting_outputs) {
				self.mark_transaction(txh, &mut marked_transactions, false);
			}

			// Do the same for the stempool
			for &txh in conflicting_stem_txs.union(&conflicting_stem_outputs) {
				self.mark_transaction(txh, &mut marked_stem_transactions, true);
			}
		}

		let freed_txs = self.sweep_transactions(marked_transactions, false);

		self.reconcile_orphans().unwrap();

		// Return something else here ?
		Ok(freed_txs)
	}

	/// The mark portion of our mark-and-sweep pool cleanup.
	///
	/// The transaction designated by conflicting_tx is immediately marked.
	/// Each output of this transaction is then examined; if a transaction in
	/// the pool spends this output and the output is not replaced by an
	/// identical output included in the updated Output set, the child is marked
	/// as well and the process continues recursively.
	///
	/// Marked transactions are added to the mutable marked_txs HashMap which
	/// is supplied by the calling function.
	fn mark_transaction(
		&self,
		conflicting_tx: hash::Hash,
		marked_txs: &mut HashSet<hash::Hash>,
		stem: bool,
	) {
		// we can stop recursively visiting txs if we have already seen this one
		if marked_txs.contains(&conflicting_tx) {
			return;
		}

		marked_txs.insert(conflicting_tx);

		if stem {
			let tx_ref = self.stem_transactions.get(&conflicting_tx);

			for output in &tx_ref.unwrap().outputs {
				match self.stempool
					.get_internal_spent_output(&output.commitment())
				{
					Some(x) => if self.blockchain.is_unspent(&x.output()).is_err() {
						self.mark_transaction(x.destination_hash().unwrap(), marked_txs, true);
					},
					None => {}
				};
			}
		} else {
			let tx_ref = self.transactions.get(&conflicting_tx);

			for output in &tx_ref.unwrap().outputs {
				match self.pool.get_internal_spent_output(&output.commitment()) {
					Some(x) => if self.blockchain.is_unspent(&x.output()).is_err() {
						self.mark_transaction(x.destination_hash().unwrap(), marked_txs, false);
					},
					None => {}
				};
			}
		}
	}
	/// The sweep portion of mark-and-sweep pool cleanup.
	///
	/// The transactions that exist in the hashmap are removed from the
	/// heap storage as well as the vertex set. Any incoming edges are removed
	/// and added to a list of freed edges. Any outbound edges are removed from
	/// both the graph and the list of freed edges. It is the responsibility of
	/// this method to ensure that the list of freed edges (inputs) are
	/// consistent.
	///
	/// TODO: There's some iteration overlap between this and the mark step.
	/// Additional bookkeeping in the mark step could optimize that away.
	fn sweep_transactions(
		&mut self,
		marked_transactions: HashSet<hash::Hash>,
		stem: bool,
	) -> Vec<transaction::Transaction> {
		let mut removed_txs = Vec::new();

		if stem {
			for tx_hash in &marked_transactions {
				let removed_tx = self.stem_transactions.remove(&tx_hash).unwrap();

				self.stempool
					.remove_pool_transaction(&removed_tx, &marked_transactions);

				removed_txs.push(removed_tx);
			}

			// final step is to update the pool to reflect the new set of roots
			// a tx that was non-root may now be root based on the txs removed
			self.stempool.update_roots();
		} else {
			for tx_hash in &marked_transactions {
				let removed_tx = self.transactions.remove(&tx_hash).unwrap();

				self.pool
					.remove_pool_transaction(&removed_tx, &marked_transactions);

				removed_txs.push(removed_tx);
			}

			// final step is to update the pool to reflect the new set of roots
			// a tx that was non-root may now be root based on the txs removed
			self.pool.update_roots();
		}
		removed_txs
	}

	/// Fetch mineable transactions.
	///
	/// Select a set of mineable transactions for block building.
	///
	/// TODO - txs have lock_heights, so possible to have "invalid" (immature)
	/// txs here?
	///
	pub fn prepare_mineable_transactions(
		&self,
		num_to_fetch: u32,
	) -> Vec<transaction::Transaction> {
		self.pool
			.get_mineable_transactions(num_to_fetch)
			.iter()
			.map(|x| self.transactions.get(x).unwrap().clone())
			.collect()
	}

	/// Remove tx from stempool
	pub fn remove_from_stempool(&mut self, tx_hash: &Hash) {
		self.stem_transactions.remove(&tx_hash);
		self.time_stem_transactions.remove(&tx_hash);
	}

	/// Whether the transaction is acceptable to the pool, given both how
	/// full the pool is and the transaction weight.
	fn is_acceptable(&self, tx: &transaction::Transaction) -> Result<(), PoolError> {
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

	// Check that the transaction is not in the stempool or in the pool
	fn check_pools(&mut self, tx_hash: &Hash, stem: bool) -> Result<(), PoolError> {
		// Check if the transaction is a stem transaction AND alreay in stempool.
		// If this is the case, we reject the transaction.
		if stem && self.stem_transactions.contains_key(&tx_hash) {
			return Err(PoolError::AlreadyInStempool);
		} else {
			// Now it leaves us with two cases:
			// 1. The transaction is not a stem transaction and is in stempool. (false &&
			// true) => The transaction has been fluffed by another node.
			// It is okay too but we have to remove this transaction from our stempool
			// before    adding it in our transaction pool
			// 2. The transaction is a stem transaction and is not in stempool. (true &&
			// false).    => Ok
			// 3. The transaction is not a stem transaction is not in stempool (false &&
			// false) => We have to check if the transaction is in the transaction
			// pool

			// Case number 1, maybe uneeded check
			if self.stem_transactions.contains_key(&tx_hash) {
				let mut tx: HashSet<hash::Hash> = HashSet::new();
				tx.insert(tx_hash.clone());
				debug!(
					LOGGER,
					"pool: check_pools: transaction has been fluffed - {}", &tx_hash,
				);
				let transaction = self.stem_transactions.remove(&tx_hash).unwrap();
				self.time_stem_transactions.remove(&tx_hash);
				self.stempool.remove_pool_transaction(&transaction, &tx);
			// Case 3
			} else if self.transactions.contains_key(&tx_hash) {
				return Err(PoolError::AlreadyInPool);
			}
		}
		Ok(())
	}

	// Check that the transaction is mature
	fn is_mature(
		&self,
		tx: &transaction::Transaction,
		head_header: &block::BlockHeader,
	) -> Result<(), PoolError> {
		if head_header.height < tx.lock_height() {
			return Err(PoolError::ImmatureTransaction {
				lock_height: tx.lock_height(),
			});
		}
		Ok(())
	}
}
