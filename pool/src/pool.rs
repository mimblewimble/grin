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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use time;
use rand;
use rand::Rng;

use core::core::hash::Hash;
use core::core::hash::Hashed;
use core::core::id::ShortIdentifiable;
use core::core::transaction;
use core::core::{OutputIdentifier, Transaction};
use core::core::{block, hash};
use util::LOGGER;
use util::secp::pedersen::Commitment;

use types::*;
pub use graph;

/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
pub struct TransactionPool<T> {
	config: PoolConfig,
	/// All transactions hash in the stempool with a time attached to ensure
	/// propagation
	pub time_stem_transactions: HashMap<hash::Hash, i64>,
	/// All transactions in the stempool
	pub stem_transactions: HashMap<hash::Hash, Box<transaction::Transaction>>,
	/// All transactions in the pool
	pub transactions: HashMap<hash::Hash, Box<transaction::Transaction>>,
	/// The stem pool
	pub stempool: Pool,
	/// The pool itself
	pub pool: Pool,
	/// Orphans in the pool
	pub orphans: Orphans,

	// blockchain is a DummyChain, for now, which mimics what the future
	// chain will offer to the pool
	blockchain: Arc<T>,
	adapter: Arc<PoolAdapter>,
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
					txs.push(*tx.clone());
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
	/// Adds a transaction to the stem memory pool, deferring to the orphans pool
	/// if necessary, and performing any connection-related validity checks.
	/// Happens under an exclusive mutable reference gated by the write portion
	/// of a RWLock.
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
					if will_stem {
						// Going to stem this transaction if parent is in stempool it's ok.
						debug!(LOGGER, "Going in stempool");
						pool_refs.push(base.with_source(Some(x)));
					} else {
						will_stem = true;
						debug!(
							LOGGER,
							"Parent is in stempool, force transaction to go in stempool"
						);
						pool_refs.push(base.with_source(Some(x)));
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
				self.stem_transactions.insert(tx_hash, Box::new(tx));
				// Track this transaction
				self.time_stem_transactions
					.insert(tx_hash, time::now_utc().to_timespec().sec);
			} else {
				// Fluff phase: transaction is added to memory pool and broadcasted normally
				self.pool.add_pool_transaction(
					pool_entry,
					blockchain_refs,
					pool_refs,
					new_unspents,
				);
				self.adapter.tx_accepted(&tx);
				self.transactions.insert(tx_hash, Box::new(tx));
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
	) -> Result<Vec<Box<transaction::Transaction>>, PoolError> {
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
	) -> Vec<Box<transaction::Transaction>> {
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
	) -> Vec<Box<transaction::Transaction>> {
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
			let mut tx_weight = -1 * (tx.inputs.len() as i32) + (4 * tx.outputs.len() as i32) + 1;
			if tx_weight < 1 {
				tx_weight = 1;
			}
			let threshold = (tx_weight as u64) * self.config.accept_fee_base;
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

#[cfg(test)]
mod tests {
	use super::*;
	use core::core::build;
	use core::global;
	use blockchain::{DummyChain, DummyChainImpl, DummyOutputSet};
	use keychain::Keychain;
	use std::sync::{Arc, RwLock};
	use blake2;
	use core::global::ChainTypes;
	use core::core::Proof;
	use core::core::hash::{Hash, Hashed};
	use core::core::pmmr::MerkleProof;
	use core::core::target::Difficulty;
	use core::core::transaction::ProofMessageElements;
	use types::PoolError::InvalidTx;

	macro_rules! expect_output_parent {
		($pool:expr, $expected:pat, $( $output:expr ),+ ) => {
			$(
				match $pool
				.search_for_best_output(
					&OutputIdentifier::from_output(&test_output($output))
				) {
					$expected => {},
					x => panic!(
						"Unexpected result from output search for {:?}, got {:?}",
						$output,
						x,
					),
				};
			)*
		}
	}

	#[test]
	/// A basic test; add a pair of transactions to the pool.
	fn test_basic_pool_add() {
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
		// We want this transaction to be rooted in the blockchain.
		let new_output = DummyOutputSet::empty()
			.with_output(test_output(5))
			.with_output(test_output(6))
			.with_output(test_output(7))
			.with_output(test_output(8));

		// Prepare a second transaction, connected to the first.
		let child_transaction = test_transaction(vec![11, 3], vec![12]);

		dummy_chain.update_output_set(new_output);

		// To mirror how this construction is intended to be used, the pool
		// is placed inside a RwLock.
		let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

		// Take the write lock and add a pool entry
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// First, add the transaction rooted in the blockchain
			let result = write_pool.add_to_memory_pool(test_source(), parent_transaction, false);
			if result.is_err() {
				panic!("got an error adding parent tx: {:?}", result.err().unwrap());
			}

			// Now, add the transaction connected as a child to the first
			let child_result =
				write_pool.add_to_memory_pool(test_source(), child_transaction, false);

			if child_result.is_err() {
				panic!(
					"got an error adding child tx: {:?}",
					child_result.err().unwrap()
				);
			}
		}

		// Now take the read lock and use a few exposed methods to check consistency
		{
			let read_pool = pool.read().unwrap();
			assert_eq!(read_pool.total_size(), 2);
			expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
			expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
			expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
			expect_output_parent!(read_pool, Parent::Unknown, 20);
		}
	}

	#[test]
	/// A basic test; add a transaction to the pool and add the child to the
	/// stempool
	fn test_pool_stempool_add() {
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
		// We want this transaction to be rooted in the blockchain.
		let new_output = DummyOutputSet::empty()
			.with_output(test_output(5))
			.with_output(test_output(6))
			.with_output(test_output(7))
			.with_output(test_output(8));

		// Prepare a second transaction, connected to the first.
		let child_transaction = test_transaction(vec![11, 3], vec![12]);

		dummy_chain.update_output_set(new_output);

		// To mirror how this construction is intended to be used, the pool
		// is placed inside a RwLock.
		let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

		// Take the write lock and add a pool entry
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// First, add the transaction rooted in the blockchain
			let result = write_pool.add_to_memory_pool(test_source(), parent_transaction, false);
			if result.is_err() {
				panic!("got an error adding parent tx: {:?}", result.err().unwrap());
			}

			// Now, add the transaction connected as a child to the first
			let child_result =
				write_pool.add_to_memory_pool(test_source(), child_transaction, true);

			if child_result.is_err() {
				panic!(
					"got an error adding child tx: {:?}",
					child_result.err().unwrap()
				);
			}
		}

		// Now take the read lock and use a few exposed methods to check consistency
		{
			let read_pool = pool.read().unwrap();
			assert_eq!(read_pool.total_size(), 2);
			if read_pool.stempool.num_transactions() == 0 {
				expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
			} else {
				expect_output_parent!(read_pool, Parent::StemPoolTransaction{tx_ref: _}, 12);
			}
			expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
			expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
			expect_output_parent!(read_pool, Parent::Unknown, 20);
		}
	}

	#[test]
	/// A basic test; add a transaction to the stempool and one the regular transaction pool
	/// Child transaction should be added to the stempool.
	fn test_stempool_pool_add() {
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		let parent_transaction = test_transaction(vec![5, 6, 7], vec![11, 3]);
		// We want this transaction to be rooted in the blockchain.
		let new_output = DummyOutputSet::empty()
			.with_output(test_output(5))
			.with_output(test_output(6))
			.with_output(test_output(7))
			.with_output(test_output(8));

		// Prepare a second transaction, connected to the first.
		let child_transaction = test_transaction(vec![11, 3], vec![12]);

		dummy_chain.update_output_set(new_output);

		// To mirror how this construction is intended to be used, the pool
		// is placed inside a RwLock.
		let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

		// Take the write lock and add a pool entry
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// First, add the transaction rooted in the blockchain
			let result = write_pool.add_to_memory_pool(test_source(), parent_transaction, true);
			if result.is_err() {
				panic!("got an error adding parent tx: {:?}", result.err().unwrap());
			}

			// Now, add the transaction connected as a child to the first
			let child_result =
				write_pool.add_to_memory_pool(test_source(), child_transaction, false);
			if child_result.is_err() {
				panic!(
					"got an error adding child tx: {:?}",
					child_result.err().unwrap()
				);
			}
		}

		// Now take the read lock and use a few exposed methods to check consistency
		{
			let read_pool = pool.read().unwrap();
			// First transaction is a stem transaction. In that case the child transaction
			// should be force stem
			assert_eq!(read_pool.total_size(), 2);
			// Parent has been directly fluffed
			if read_pool.stempool.num_transactions() == 0 {
				expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
			} else {
				expect_output_parent!(read_pool, Parent::StemPoolTransaction{tx_ref: _}, 12);
			}
			expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
			expect_output_parent!(read_pool, Parent::BlockTransaction, 8);
			expect_output_parent!(read_pool, Parent::Unknown, 20);
		}
	}

	#[test]
	/// Testing various expected error conditions
	pub fn test_pool_add_error() {
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		let new_output = DummyOutputSet::empty()
			.with_output(test_output(5))
			.with_output(test_output(6))
			.with_output(test_output(7));

		dummy_chain.update_output_set(new_output);

		let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// First expected failure: duplicate output
			let duplicate_tx = test_transaction(vec![5, 6], vec![7]);

			match write_pool.add_to_memory_pool(test_source(), duplicate_tx, false) {
				Ok(_) => panic!("Got OK from add_to_memory_pool when dup was expected"),
				Err(x) => {
					match x {
						PoolError::DuplicateOutput {
							other_tx,
							in_chain,
							output,
						} => if other_tx.is_some() || !in_chain
							|| output != test_output(7).commitment()
						{
							panic!("Unexpected parameter in DuplicateOutput: {:?}", x);
						},
						_ => panic!(
							"Unexpected error when adding duplicate output transaction: {:?}",
							x
						),
					};
				}
			};

			// To test DoubleSpend and AlreadyInPool conditions, we need to add
			// a valid transaction.
			let valid_transaction = test_transaction(vec![5, 6], vec![9]);

			match write_pool.add_to_memory_pool(test_source(), valid_transaction.clone(), false) {
				Ok(_) => {}
				Err(x) => panic!("Unexpected error while adding a valid transaction: {:?}", x),
			};

			// Now, test a DoubleSpend by consuming the same blockchain unspent
			// as valid_transaction:
			let double_spend_transaction = test_transaction(vec![6], vec![2]);

			match write_pool.add_to_memory_pool(test_source(), double_spend_transaction, false) {
				Ok(_) => panic!("Expected error when adding double spend, got Ok"),
				Err(x) => {
					match x {
						PoolError::DoubleSpend {
							other_tx: _,
							spent_output,
						} => if spent_output != test_output(6).commitment() {
							panic!("Unexpected parameter in DoubleSpend: {:?}", x);
						},
						_ => panic!(
							"Unexpected error when adding double spend transaction: {:?}",
							x
						),
					};
				}
			};

			// Note, this used to work as expected, but after aggsig implementation
			// creating another transaction with the same inputs/outputs doesn't create
			// the same hash ID due to the random nonces in an aggsig. This
			// will instead throw a (correct as well) Already spent error. An AlreadyInPool
			// error can only come up in the case of the exact same transaction being
			// added
			//let already_in_pool = test_transaction(vec![5, 6], vec![9]);

			match write_pool.add_to_memory_pool(test_source(), valid_transaction, false) {
				Ok(_) => panic!("Expected error when adding already in pool, got Ok"),
				Err(x) => {
					match x {
						PoolError::AlreadyInPool => {}
						_ => panic!("Unexpected error when adding already in pool tx: {:?}", x),
					};
				}
			};

			assert_eq!(write_pool.total_size(), 1);

			// now attempt to add a timelocked tx to the pool
			// should fail as invalid based on current height
			let timelocked_tx_1 = timelocked_transaction(vec![9], vec![5], 10);
			match write_pool.add_to_memory_pool(test_source(), timelocked_tx_1, false) {
				Err(PoolError::ImmatureTransaction {
					lock_height: height,
				}) => {
					assert_eq!(height, 10);
				}
				Err(e) => panic!("expected ImmatureTransaction error here - {:?}", e),
				Ok(_) => panic!("expected ImmatureTransaction error here"),
			};
		}
	}

	#[test]
	fn test_immature_coinbase() {
		global::set_mining_mode(ChainTypes::AutomatedTesting);
		let mut dummy_chain = DummyChainImpl::new();
		let proof_size = global::proofsize();

		let lock_height = 1 + global::coinbase_maturity();
		assert_eq!(lock_height, 4);

		let coinbase_output = test_coinbase_output(15);
		dummy_chain.update_output_set(DummyOutputSet::empty().with_output(coinbase_output));

		let chain_ref = Arc::new(dummy_chain);
		let pool = RwLock::new(test_setup(&chain_ref));

		{
			let mut write_pool = pool.write().unwrap();

			let coinbase_header = block::BlockHeader {
				height: 1,
				pow: Proof::random(proof_size),
				..block::BlockHeader::default()
			};
			chain_ref.store_head_header(&coinbase_header);

			let head_header = block::BlockHeader {
				height: 2,
				pow: Proof::random(proof_size),
				..block::BlockHeader::default()
			};
			chain_ref.store_head_header(&head_header);

			let txn = test_transaction_with_coinbase_input(15, coinbase_header.hash(), vec![10, 3]);
			let result = write_pool.add_to_memory_pool(test_source(), txn, false);
			match result {
				Err(InvalidTx(transaction::Error::ImmatureCoinbase)) => {}
				_ => panic!("expected ImmatureCoinbase error here"),
			};

			let head_header = block::BlockHeader {
				height: 4,
				..block::BlockHeader::default()
			};
			chain_ref.store_head_header(&head_header);

			let txn = test_transaction_with_coinbase_input(15, coinbase_header.hash(), vec![10, 3]);
			let result = write_pool.add_to_memory_pool(test_source(), txn, false);
			match result {
				Ok(_) => {}
				Err(_) => panic!("this should not return an error here"),
			};
		}
	}

	#[test]
	/// Testing an expected orphan
	fn test_add_orphan() {
		// TODO we need a test here
	}

	#[test]
	fn test_zero_confirmation_reconciliation() {
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		// single Output
		let new_output = DummyOutputSet::empty().with_output(test_output(100));

		dummy_chain.update_output_set(new_output);
		let chain_ref = Arc::new(dummy_chain);
		let pool = RwLock::new(test_setup(&chain_ref));

		// now create two txs
		// tx1 spends the Output
		// tx2 spends output from tx1
		let tx1 = test_transaction(vec![100], vec![90]);
		let tx2 = test_transaction(vec![90], vec![80]);

		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// now add both txs to the pool (tx2 spends tx1 with zero confirmations)
			// both should be accepted if tx1 added before tx2
			write_pool
				.add_to_memory_pool(test_source(), tx1, false)
				.unwrap();
			write_pool
				.add_to_memory_pool(test_source(), tx2, false)
				.unwrap();

			assert_eq!(write_pool.pool_size(), 2);
		}

		let txs: Vec<transaction::Transaction>;
		{
			let read_pool = pool.read().unwrap();
			let mut mineable_txs = read_pool.prepare_mineable_transactions(3);
			txs = mineable_txs.drain(..).map(|x| *x).collect();

			// confirm we can preparing both txs for mining here
			// one root tx in the pool, and one non-root vertex in the pool
			assert_eq!(txs.len(), 2);
		}

		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		// now "mine" the block passing in the mineable txs from earlier
		let block = block::Block::new(
			&block::BlockHeader::default(),
			txs.iter().collect(),
			&keychain,
			&key_id,
			Difficulty::one(),
		).unwrap();

		// now apply the block to ensure the chainstate is updated before we reconcile
		chain_ref.apply_block(&block);

		// now reconcile the block
		// we should evict both txs here
		{
			let mut write_pool = pool.write().unwrap();
			let evicted_transactions = write_pool.reconcile_block(&block).unwrap();
			assert_eq!(evicted_transactions.len(), 2);
		}

		// check the pool is consistent after reconciling the block
		// we should have zero txs in the pool (neither roots nor non-roots)
		{
			let read_pool = pool.write().unwrap();
			assert_eq!(read_pool.pool.len_vertices(), 0);
			assert_eq!(read_pool.pool.len_roots(), 0);
		}
	}

	#[test]
	/// Testing block reconciliation
	fn test_block_reconciliation() {
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		let new_output = DummyOutputSet::empty()
			.with_output(test_output(10))
			.with_output(test_output(20))
			.with_output(test_output(30))
			.with_output(test_output(40));

		dummy_chain.update_output_set(new_output);

		let chain_ref = Arc::new(dummy_chain);

		let pool = RwLock::new(test_setup(&chain_ref));

		// Preparation: We will introduce a three root pool transactions.
		// 1. A transaction that should be invalidated because it is exactly
		//  contained in the block.
		// 2. A transaction that should be invalidated because the input is
		//  consumed in the block, although it is not exactly consumed.
		// 3. A transaction that should remain after block reconciliation.
		let block_transaction = test_transaction(vec![10], vec![8]);
		let conflict_transaction = test_transaction(vec![20], vec![12, 6]);
		let valid_transaction = test_transaction(vec![30], vec![13, 15]);

		// We will also introduce a few children:
		// 4. A transaction that descends from transaction 1, that is in
		//  turn exactly contained in the block.
		let block_child = test_transaction(vec![8], vec![5, 1]);
		// 5. A transaction that descends from transaction 4, that is not
		//  contained in the block at all and should be valid after
		//  reconciliation.
		let pool_child = test_transaction(vec![5], vec![3]);
		// 6. A transaction that descends from transaction 2 that does not
		//  conflict with anything in the block in any way, but should be
		//  invalidated (orphaned).
		let conflict_child = test_transaction(vec![12], vec![2]);
		// 7. A transaction that descends from transaction 2 that should be
		//  valid due to its inputs being satisfied by the block.
		let conflict_valid_child = test_transaction(vec![6], vec![4]);
		// 8. A transaction that descends from transaction 3 that should be
		//  invalidated due to an output conflict.
		let valid_child_conflict = test_transaction(vec![13], vec![9]);
		// 9. A transaction that descends from transaction 3 that should remain
		//  valid after reconciliation.
		let valid_child_valid = test_transaction(vec![15], vec![11]);
		// 10. A transaction that descends from both transaction 6 and
		//  transaction 9
		let mixed_child = test_transaction(vec![2, 11], vec![7]);

		// Add transactions.
		// Note: There are some ordering constraints that must be followed here
		// until orphans is 100% implemented. Once the orphans process has
		// stabilized, we can mix these up to exercise that path a bit.
		let mut txs_to_add = vec![
			block_transaction,
			conflict_transaction,
			valid_transaction,
			block_child,
			pool_child,
			conflict_child,
			conflict_valid_child,
			valid_child_conflict,
			valid_child_valid,
			mixed_child,
		];

		let expected_pool_size = txs_to_add.len();

		// First we add the above transactions to the pool; all should be
		// accepted.
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			for tx in txs_to_add.drain(..) {
				write_pool
					.add_to_memory_pool(test_source(), tx, false)
					.unwrap();
			}

			assert_eq!(write_pool.total_size(), expected_pool_size);
		}
		// Now we prepare the block that will cause the above condition.
		// First, the transactions we want in the block:
		// - Copy of 1
		let block_tx_1 = test_transaction(vec![10], vec![8]);
		// - Conflict w/ 2, satisfies 7
		let block_tx_2 = test_transaction(vec![20], vec![6]);
		// - Copy of 4
		let block_tx_3 = test_transaction(vec![8], vec![5, 1]);
		// - Output conflict w/ 8
		let block_tx_4 = test_transaction(vec![40], vec![9, 1]);
		let block_transactions = vec![&block_tx_1, &block_tx_2, &block_tx_3, &block_tx_4];

		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();

		let block = block::Block::new(
			&block::BlockHeader::default(),
			block_transactions,
			&keychain,
			&key_id,
			Difficulty::one(),
		).unwrap();

		chain_ref.apply_block(&block);

		// Block reconciliation
		{
			let mut write_pool = pool.write().unwrap();

			let evicted_transactions = write_pool.reconcile_block(&block);

			assert!(evicted_transactions.is_ok());

			assert_eq!(evicted_transactions.unwrap().len(), 6);

			// TODO: Txids are not yet deterministic. When they are, we should
			// check the specific transactions that were evicted.
		}

		// Using the pool's methods to validate a few end conditions.
		{
			let read_pool = pool.read().unwrap();

			assert_eq!(read_pool.total_size(), 4);

			// We should have available blockchain outputs
			expect_output_parent!(read_pool, Parent::BlockTransaction, 9, 1);

			// We should have spent blockchain outputs
			expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 5, 6);

			// We should have spent pool references
			expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 15);

			// We should have unspent pool references
			expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 3, 11, 13);

			// References internal to the block should be unknown
			expect_output_parent!(read_pool, Parent::Unknown, 8);

			// Evicted transactions should have unknown outputs
			expect_output_parent!(read_pool, Parent::Unknown, 2, 7);
		}
	}

	#[test]
	/// Test transaction selection and block building.
	fn test_block_building() {
		// Add a handful of transactions
		let mut dummy_chain = DummyChainImpl::new();
		let head_header = block::BlockHeader {
			height: 1,
			..block::BlockHeader::default()
		};
		dummy_chain.store_head_header(&head_header);

		let new_output = DummyOutputSet::empty()
			.with_output(test_output(10))
			.with_output(test_output(20))
			.with_output(test_output(30))
			.with_output(test_output(40));

		dummy_chain.update_output_set(new_output);

		let chain_ref = Arc::new(dummy_chain);

		let pool = RwLock::new(test_setup(&chain_ref));

		let root_tx_1 = test_transaction(vec![10, 20], vec![24]);
		let root_tx_2 = test_transaction(vec![30], vec![28]);
		let root_tx_3 = test_transaction(vec![40], vec![38]);

		let child_tx_1 = test_transaction(vec![24], vec![22]);
		let child_tx_2 = test_transaction(vec![38], vec![32]);

		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			assert!(
				write_pool
					.add_to_memory_pool(test_source(), root_tx_1, false)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), root_tx_2, false)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), root_tx_3, false)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), child_tx_1, false)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), child_tx_2, false)
					.is_ok()
			);

			assert_eq!(write_pool.total_size(), 5);
		}

		// Request blocks
		let block: block::Block;
		let mut txs: Vec<Box<transaction::Transaction>>;
		{
			let read_pool = pool.read().unwrap();
			txs = read_pool.prepare_mineable_transactions(3);
			assert_eq!(txs.len(), 3);
			// TODO: This is ugly, either make block::new take owned
			// txs instead of mut refs, or change
			// prepare_mineable_transactions to return mut refs
			let block_txs: Vec<transaction::Transaction> = txs.drain(..).map(|x| *x).collect();
			let tx_refs = block_txs.iter().collect();

			let keychain = Keychain::from_random_seed().unwrap();
			let key_id = keychain.derive_key_id(1).unwrap();
			block = block::Block::new(
				&block::BlockHeader::default(),
				tx_refs,
				&keychain,
				&key_id,
				Difficulty::one(),
			).unwrap();
		}

		chain_ref.apply_block(&block);
		// Reconcile block
		{
			let mut write_pool = pool.write().unwrap();

			let evicted_transactions = write_pool.reconcile_block(&block);

			assert!(evicted_transactions.is_ok());

			assert_eq!(evicted_transactions.unwrap().len(), 3);
			assert_eq!(write_pool.total_size(), 2);
		}
	}

	fn test_setup(dummy_chain: &Arc<DummyChainImpl>) -> TransactionPool<DummyChainImpl> {
		TransactionPool {
			config: PoolConfig {
				accept_fee_base: 0,
				max_pool_size: 10_000,
				dandelion_probability: 90,
				dandelion_embargo: 30,
			},
			time_stem_transactions: HashMap::new(),
			stem_transactions: HashMap::new(),
			transactions: HashMap::new(),
			stempool: Pool::empty(),
			pool: Pool::empty(),
			orphans: Orphans::empty(),
			blockchain: dummy_chain.clone(),
			adapter: Arc::new(NoopAdapter {}),
		}
	}

	/// Cobble together a test transaction for testing the transaction pool.
	///
	/// Connectivity here is the most important element.
	/// Every output is given a blinding key equal to its value, so that the
	/// entire commitment can be derived deterministically from just the value.
	///
	/// Fees are the remainder between input and output values,
	/// so the numbers should make sense.
	fn test_transaction(
		input_values: Vec<u64>,
		output_values: Vec<u64>,
	) -> transaction::Transaction {
		let keychain = keychain_for_tests();

		let input_sum = input_values.iter().sum::<u64>() as i64;
		let output_sum = output_values.iter().sum::<u64>() as i64;

		let fees: i64 = input_sum - output_sum;
		assert!(fees >= 0);

		let mut tx_elements = Vec::new();

		for input_value in input_values {
			let key_id = keychain.derive_key_id(input_value as u32).unwrap();
			tx_elements.push(build::input(input_value, key_id));
		}

		for output_value in output_values {
			let key_id = keychain.derive_key_id(output_value as u32).unwrap();
			tx_elements.push(build::output(output_value, key_id));
		}
		tx_elements.push(build::with_fee(fees as u64));

		build::transaction(tx_elements, &keychain).unwrap()
	}

	fn test_transaction_with_coinbase_input(
		input_value: u64,
		input_block_hash: Hash,
		output_values: Vec<u64>,
	) -> transaction::Transaction {
		let keychain = keychain_for_tests();

		let output_sum = output_values.iter().sum::<u64>() as i64;

		let fees: i64 = input_value as i64 - output_sum;
		assert!(fees >= 0);

		let mut tx_elements = Vec::new();

		let merkle_proof = MerkleProof {
			node: Hash::default(),
			root: Hash::default(),
			peaks: vec![Hash::default()],
			..MerkleProof::default()
		};

		let key_id = keychain.derive_key_id(input_value as u32).unwrap();
		tx_elements.push(build::coinbase_input(
			input_value,
			input_block_hash,
			merkle_proof,
			key_id,
		));

		for output_value in output_values {
			let key_id = keychain.derive_key_id(output_value as u32).unwrap();
			tx_elements.push(build::output(output_value, key_id));
		}
		tx_elements.push(build::with_fee(fees as u64));

		build::transaction(tx_elements, &keychain).unwrap()
	}

	/// Very un-dry way of building a vanilla tx and adding a lock_height to it.
	/// TODO - rethink this.
	fn timelocked_transaction(
		input_values: Vec<u64>,
		output_values: Vec<u64>,
		lock_height: u64,
	) -> transaction::Transaction {
		let keychain = keychain_for_tests();

		let fees: i64 =
			input_values.iter().sum::<u64>() as i64 - output_values.iter().sum::<u64>() as i64;
		assert!(fees >= 0);

		let mut tx_elements = Vec::new();

		for input_value in input_values {
			let key_id = keychain.derive_key_id(input_value as u32).unwrap();
			tx_elements.push(build::input(input_value, key_id));
		}

		for output_value in output_values {
			let key_id = keychain.derive_key_id(output_value as u32).unwrap();
			tx_elements.push(build::output(output_value, key_id));
		}
		tx_elements.push(build::with_fee(fees as u64));

		tx_elements.push(build::with_lock_height(lock_height));
		build::transaction(tx_elements, &keychain).unwrap()
	}

	/// Deterministically generate an output defined by our test scheme
	fn test_output(value: u64) -> transaction::Output {
		let keychain = keychain_for_tests();
		let key_id = keychain.derive_key_id(value as u32).unwrap();
		let msg = ProofMessageElements::new(value, &key_id);
		let commit = keychain.commit(value, &key_id).unwrap();
		let proof = keychain
			.range_proof(value, &key_id, commit, None, msg.to_proof_message())
			.unwrap();
		transaction::Output {
			features: transaction::OutputFeatures::DEFAULT_OUTPUT,
			commit: commit,
			proof: proof,
		}
	}

	/// Deterministically generate a coinbase output defined by our test scheme
	fn test_coinbase_output(value: u64) -> transaction::Output {
		let keychain = keychain_for_tests();
		let key_id = keychain.derive_key_id(value as u32).unwrap();
		let msg = ProofMessageElements::new(value, &key_id);
		let commit = keychain.commit(value, &key_id).unwrap();
		let proof = keychain
			.range_proof(value, &key_id, commit, None, msg.to_proof_message())
			.unwrap();
		transaction::Output {
			features: transaction::OutputFeatures::COINBASE_OUTPUT,
			commit: commit,
			proof: proof,
		}
	}

	fn keychain_for_tests() -> Keychain {
		let seed = "pool_tests";
		let seed = blake2::blake2b::blake2b(32, &[], seed.as_bytes());
		Keychain::from_seed(seed.as_bytes()).unwrap()
	}

	/// A generic TxSource representing a test
	fn test_source() -> TxSource {
		TxSource {
			debug_name: "test".to_string(),
			identifier: "127.0.0.1".to_string(),
		}
	}
}
