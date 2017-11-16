// Copyright 2017 The Grin Developers
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

use types::*;
pub use graph;

use core::core::transaction;
use core::core::block;
use core::core::hash;
use core::global;

use util::secp::pedersen::Commitment;

use std::sync::Arc;
use std::collections::{HashMap, HashSet};

/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
pub struct TransactionPool<T> {
	config: PoolConfig,
	/// All transactions in the pool
	pub transactions: HashMap<hash::Hash, Box<transaction::Transaction>>,
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
			transactions: HashMap::new(),
			pool: Pool::empty(),
			orphans: Orphans::empty(),
			blockchain: chain,
			adapter: adapter,
		}
	}

	/// Searches for an output, designated by its commitment, from the current
	/// best UTXO view, presented by taking the best blockchain UTXO set (as
	/// determined by the blockchain component) and rectifying pool spent and
	/// unspents.
	/// Detects double spends and unknown references from the pool and
	/// blockchain only; any conflicts with entries in the orphans set must
	/// be accounted for separately, if relevant.
	pub fn search_for_best_output(&self, output_commitment: &Commitment) -> Parent {
		// The current best unspent set is:
  //   Pool unspent + (blockchain unspent - pool->blockchain spent)
  // Pool unspents are unconditional so we check those first
		self.pool
			.get_available_output(output_commitment)
			.map(|x| {
				Parent::PoolTransaction {
					tx_ref: x.source_hash().unwrap(),
				}
			})
			.or(self.search_blockchain_unspents(output_commitment))
			.or(self.search_pool_spents(output_commitment))
			.unwrap_or(Parent::Unknown)
	}

	// search_blockchain_unspents searches the current view of the blockchain
 // unspent set, represented by blockchain unspents - pool spents, for an
 // output designated by output_commitment.
	fn search_blockchain_unspents(&self, output_commitment: &Commitment) -> Option<Parent> {
		self.blockchain
			.get_unspent(output_commitment)
			.ok()
			.map(|output| {
				match self.pool.get_blockchain_spent(output_commitment) {
					Some(x) => Parent::AlreadySpent {
						other_tx: x.destination_hash().unwrap(),
					},
					None => Parent::BlockTransaction { output },
				}
			})
	}

	// search_pool_spents is the second half of pool input detection, after the
 // available_outputs have been checked. This returns either a
 // Parent::AlreadySpent or None.
	fn search_pool_spents(&self, output_commitment: &Commitment) -> Option<Parent> {
		self.pool.get_internal_spent(output_commitment).map(|x| {
			Parent::AlreadySpent {
				other_tx: x.destination_hash().unwrap(),
			}
		})
	}

	/// Get the number of transactions in the pool
	pub fn pool_size(&self) -> usize {
		self.pool.num_transactions()
	}

	/// Get the number of orphans in the pool
	pub fn orphans_size(&self) -> usize {
		self.orphans.num_transactions()
	}

	/// Get the total size (transactions + orphans) of the pool
	pub fn total_size(&self) -> usize {
		self.pool.num_transactions() + self.orphans.num_transactions()
	}

	/// Attempts to add a transaction to the pool.
	///
	/// Adds a transaction to the memory pool, deferring to the orphans pool
	/// if necessary, and performing any connection-related validity checks.
	/// Happens under an exclusive mutable reference gated by the write portion
	/// of a RWLock.
	pub fn add_to_memory_pool(
		&mut self,
		_: TxSource,
		tx: transaction::Transaction,
	) -> Result<(), PoolError> {
		// Do we have the capacity to accept this transaction?
		if let Err(e) = self.is_acceptable(&tx) {
			return Err(e);
		}

		// Making sure the transaction is valid before anything else.
		tx.validate().map_err(|_e| PoolError::Invalid)?;

		// The first check involves ensuring that an identical transaction is
  // not already in the pool's transaction set.
  // A non-authoritative similar check should be performed under the
  // pool's read lock before we get to this point, which would catch the
  // majority of duplicate cases. The race condition is caught here.
  // TODO: When the transaction identifier is finalized, the assumptions
  // here may change depending on the exact coverage of the identifier.
  // The current tx.hash() method, for example, does not cover changes
  // to fees or other elements of the signature preimage.
		let tx_hash = graph::transaction_identifier(&tx);
		if self.transactions.contains_key(&tx_hash) {
			return Err(PoolError::AlreadyInPool);
		}

		let head_header = self.blockchain.head_header()?;
		if head_header.height < tx.lock_height {
			return Err(PoolError::ImmatureTransaction {
				lock_height: tx.lock_height,
			});
		}

		// The next issue is to identify all unspent outputs that
  // this transaction will consume and make sure they exist in the set.
		let mut pool_refs: Vec<graph::Edge> = Vec::new();
		let mut orphan_refs: Vec<graph::Edge> = Vec::new();
		let mut blockchain_refs: Vec<graph::Edge> = Vec::new();

		for input in &tx.inputs {
			let base = graph::Edge::new(None, Some(tx_hash), input.commitment());

			// Note that search_for_best_output does not examine orphans, by
   // design. If an incoming transaction consumes pool outputs already
   // spent by the orphans set, this does not preclude its inclusion
   // into the pool.
			match self.search_for_best_output(&input.commitment()) {
				Parent::PoolTransaction { tx_ref: x } => pool_refs.push(base.with_source(Some(x))),
				Parent::BlockTransaction { output } => {
					// TODO - pull this out into a separate function?
					if output.features.contains(transaction::COINBASE_OUTPUT) {
						if let Ok(out_header) = self.blockchain
							.get_block_header_by_output_commit(&output.commitment())
						{
							let lock_height = out_header.height + global::coinbase_maturity();
							if head_header.height < lock_height {
								return Err(PoolError::ImmatureCoinbase {
									header: out_header,
									output: output.commitment(),
								});
							};
						};
					};
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
			.map(|x| graph::Edge::new(Some(tx_hash), None, x.commitment()))
			.collect();

		if !is_orphan {
			// In the non-orphan (pool) case, we've ensured that every input
   // maps one-to-one with an unspent (available) output, and each
   // output is unique. No further checks are necessary.
			self.pool
				.add_pool_transaction(pool_entry, blockchain_refs, pool_refs, new_unspents);

			self.reconcile_orphans().unwrap();
			self.adapter.tx_accepted(&tx);
			self.transactions.insert(tx_hash, Box::new(tx));
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
		if self.blockchain.get_unspent(&output.commitment()).is_ok() {
			return Err(PoolError::DuplicateOutput {
				other_tx: None,
				in_chain: true,
				output: output.commitment(),
			});
		}


		// Check for existence of this output in the pool
		match self.pool.find_output(&output.commitment()) {
			Some(x) => {
				return Err(PoolError::DuplicateOutput {
					other_tx: Some(x),
					in_chain: false,
					output: output.commitment(),
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
	pub fn reconcile_block(
		&mut self,
		block: &block::Block,
	) -> Result<Vec<Box<transaction::Transaction>>, PoolError> {
		// If this pool has been kept in sync correctly, serializing all
  // updates, then the inputs must consume only members of the blockchain
  // utxo set.
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
  // fully satisfied by outputs in the updated utxo view (after
  // reconciliation of the block), or there are no more children.
  //
  // Additionally, to protect our invariant dictating no duplicate
  // outputs, each output generated by the new utxo set is checked
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

		{
			// find all conflicting txs based on inputs to the block
			let conflicting_txs: HashSet<hash::Hash> = block
				.inputs
				.iter()
				.filter_map(|x| self.pool.get_external_spent_output(&x.commitment()))
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

			// now iterate over all conflicting hashes from both txs and outputs
   // we can just use the union of the two sets here to remove duplicates
			for &txh in conflicting_txs.union(&conflicting_outputs) {
				self.mark_transaction(txh, &mut marked_transactions);
			}
		}
		let freed_txs = self.sweep_transactions(marked_transactions);

		self.reconcile_orphans().unwrap();

		Ok(freed_txs)
	}

	/// The mark portion of our mark-and-sweep pool cleanup.
	///
	/// The transaction designated by conflicting_tx is immediately marked.
	/// Each output of this transaction is then examined; if a transaction in
	/// the pool spends this output and the output is not replaced by an
	/// identical output included in the updated UTXO set, the child is marked
	/// as well and the process continues recursively.
	///
	/// Marked transactions are added to the mutable marked_txs HashMap which
	/// is supplied by the calling function.
	fn mark_transaction(&self, conflicting_tx: hash::Hash, marked_txs: &mut HashSet<hash::Hash>) {
		// we can stop recursively visiting txs if we have already seen this one
		if marked_txs.contains(&conflicting_tx) {
			return;
		}

		marked_txs.insert(conflicting_tx);

		let tx_ref = self.transactions.get(&conflicting_tx);

		for output in &tx_ref.unwrap().outputs {
			match self.pool.get_internal_spent_output(&output.commitment()) {
				Some(x) => if self.blockchain.get_unspent(&x.output_commitment()).is_err() {
					self.mark_transaction(x.destination_hash().unwrap(), marked_txs);
				},
				None => {}
			};
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
	) -> Vec<Box<transaction::Transaction>> {
		let mut removed_txs = Vec::new();

		for tx_hash in &marked_transactions {
			let removed_tx = self.transactions.remove(&tx_hash).unwrap();

			self.pool
				.remove_pool_transaction(&removed_tx, &marked_transactions);

			removed_txs.push(removed_tx);
		}

		// final step is to update the pool to reflect the new set of roots
  // a tx that was non-root may now be root based on the txs removed
		self.pool.update_roots();

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
			if tx.fee < threshold {
				return Err(PoolError::LowFeeTransaction(threshold));
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use core::core::build;
	use blockchain::{DummyChain, DummyChainImpl, DummyUtxoSet};
	use util::secp;
	use keychain::Keychain;
	use std::sync::{Arc, RwLock};
	use blake2;
	use core::global::ChainTypes;
	use core::core::SwitchCommitHash;

	macro_rules! expect_output_parent {
		($pool:expr, $expected:pat, $( $output:expr ),+ ) => {
			$(
				match $pool.search_for_best_output(&test_output($output).commitment()) {
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
		let new_utxo = DummyUtxoSet::empty()
			.with_output(test_output(5))
			.with_output(test_output(6))
			.with_output(test_output(7))
			.with_output(test_output(8));

		// Prepare a second transaction, connected to the first.
		let child_transaction = test_transaction(vec![11, 3], vec![12]);

		dummy_chain.update_utxo_set(new_utxo);

		// To mirror how this construction is intended to be used, the pool
  // is placed inside a RwLock.
		let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));

		// Take the write lock and add a pool entry
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// First, add the transaction rooted in the blockchain
			let result = write_pool.add_to_memory_pool(test_source(), parent_transaction);
			if result.is_err() {
				panic!("got an error adding parent tx: {:?}", result.err().unwrap());
			}

			// Now, add the transaction connected as a child to the first
			let child_result = write_pool.add_to_memory_pool(test_source(), child_transaction);

			if child_result.is_err() {
				panic!(
					"got an error adding child tx: {:?}",
					child_result.err().unwrap()
				);
			}
		}

		// Now take the read lock and use a few exposed methods to check
  // consistency
		{
			let read_pool = pool.read().unwrap();
			assert_eq!(read_pool.total_size(), 2);
			expect_output_parent!(read_pool, Parent::PoolTransaction{tx_ref: _}, 12);
			expect_output_parent!(read_pool, Parent::AlreadySpent{other_tx: _}, 11, 5);
			expect_output_parent!(read_pool, Parent::BlockTransaction{output: _}, 8);
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

		let new_utxo = DummyUtxoSet::empty()
			.with_output(test_output(5))
			.with_output(test_output(6))
			.with_output(test_output(7));

		dummy_chain.update_utxo_set(new_utxo);

		let pool = RwLock::new(test_setup(&Arc::new(dummy_chain)));
		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// First expected failure: duplicate output
			let duplicate_tx = test_transaction(vec![5, 6], vec![7]);

			match write_pool.add_to_memory_pool(test_source(), duplicate_tx) {
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

			match write_pool.add_to_memory_pool(test_source(), valid_transaction) {
				Ok(_) => {}
				Err(x) => panic!("Unexpected error while adding a valid transaction: {:?}", x),
			};

			// Now, test a DoubleSpend by consuming the same blockchain unspent
   // as valid_transaction:
			let double_spend_transaction = test_transaction(vec![6], vec![2]);

			match write_pool.add_to_memory_pool(test_source(), double_spend_transaction) {
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

			let already_in_pool = test_transaction(vec![5, 6], vec![9]);

			match write_pool.add_to_memory_pool(test_source(), already_in_pool) {
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
			match write_pool.add_to_memory_pool(test_source(), timelocked_tx_1) {
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
		let coinbase_output = test_coinbase_output(15);
		dummy_chain.update_utxo_set(DummyUtxoSet::empty().with_output(coinbase_output));

		let chain_ref = Arc::new(dummy_chain);
		let pool = RwLock::new(test_setup(&chain_ref));

		{
			let mut write_pool = pool.write().unwrap();

			let coinbase_header = block::BlockHeader {
				height: 1,
				..block::BlockHeader::default()
			};
			chain_ref
				.store_header_by_output_commitment(coinbase_output.commitment(), &coinbase_header);

			let head_header = block::BlockHeader {
				height: 2,
				..block::BlockHeader::default()
			};
			chain_ref.store_head_header(&head_header);

			let txn = test_transaction(vec![15], vec![10, 3]);
			let result = write_pool.add_to_memory_pool(test_source(), txn);
			match result {
				Err(PoolError::ImmatureCoinbase {
					header: _,
					output: out,
				}) => {
					assert_eq!(out, coinbase_output.commitment());
				}
				_ => panic!("expected ImmatureCoinbase error here"),
			};

			let head_header = block::BlockHeader {
				height: 3,
				..block::BlockHeader::default()
			};
			chain_ref.store_head_header(&head_header);

			let txn = test_transaction(vec![15], vec![10, 3]);
			let result = write_pool.add_to_memory_pool(test_source(), txn);
			match result {
				Err(PoolError::ImmatureCoinbase {
					header: _,
					output: out,
				}) => {
					assert_eq!(out, coinbase_output.commitment());
				}
				_ => panic!("expected ImmatureCoinbase error here"),
			};

			let head_header = block::BlockHeader {
				height: 5,
				..block::BlockHeader::default()
			};
			chain_ref.store_head_header(&head_header);

			let txn = test_transaction(vec![15], vec![10, 3]);
			let result = write_pool.add_to_memory_pool(test_source(), txn);
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

		// single UTXO
		let new_utxo = DummyUtxoSet::empty().with_output(test_output(100));

		dummy_chain.update_utxo_set(new_utxo);
		let chain_ref = Arc::new(dummy_chain);
		let pool = RwLock::new(test_setup(&chain_ref));

		// now create two txs
  // tx1 spends the UTXO
  // tx2 spends output from tx1
		let tx1 = test_transaction(vec![100], vec![90]);
		let tx2 = test_transaction(vec![90], vec![80]);

		{
			let mut write_pool = pool.write().unwrap();
			assert_eq!(write_pool.total_size(), 0);

			// now add both txs to the pool (tx2 spends tx1 with zero confirmations)
   // both should be accepted if tx1 added before tx2
			write_pool.add_to_memory_pool(test_source(), tx1).unwrap();
			write_pool.add_to_memory_pool(test_source(), tx2).unwrap();

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

		let new_utxo = DummyUtxoSet::empty()
			.with_output(test_output(10))
			.with_output(test_output(20))
			.with_output(test_output(30))
			.with_output(test_output(40));

		dummy_chain.update_utxo_set(new_utxo);

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
				write_pool.add_to_memory_pool(test_source(), tx).unwrap();
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
			expect_output_parent!(read_pool, Parent::BlockTransaction{output: _}, 9, 1);

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

		let new_utxo = DummyUtxoSet::empty()
			.with_output(test_output(10))
			.with_output(test_output(20))
			.with_output(test_output(30))
			.with_output(test_output(40));

		dummy_chain.update_utxo_set(new_utxo);

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
					.add_to_memory_pool(test_source(), root_tx_1)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), root_tx_2)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), root_tx_3)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), child_tx_1)
					.is_ok()
			);
			assert!(
				write_pool
					.add_to_memory_pool(test_source(), child_tx_2)
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
			block = block::Block::new(&block::BlockHeader::default(), tx_refs, &keychain, &key_id)
				.unwrap();
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
			},
			transactions: HashMap::new(),
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
	/// Fees are the remainder between input and output values, so the numbers
	/// should make sense.
	fn test_transaction(
		input_values: Vec<u64>,
		output_values: Vec<u64>,
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

		let (tx, _) = build::transaction(tx_elements, &keychain).unwrap();
		tx
	}

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
		let (tx, _) = build::transaction(tx_elements, &keychain).unwrap();
		tx
	}

	/// Deterministically generate an output defined by our test scheme
	fn test_output(value: u64) -> transaction::Output {
		let keychain = keychain_for_tests();
		let key_id = keychain.derive_key_id(value as u32).unwrap();
		let commit = keychain.commit(value, &key_id).unwrap();
		let switch_commit = keychain.switch_commit(&key_id).unwrap();
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
		let msg = secp::pedersen::ProofMessage::empty();
		let proof = keychain.range_proof(value, &key_id, commit, msg).unwrap();

		transaction::Output {
			features: transaction::DEFAULT_OUTPUT,
			commit: commit,
			switch_commit_hash: switch_commit_hash,
			proof: proof,
		}
	}

	/// Deterministically generate a coinbase output defined by our test scheme
	fn test_coinbase_output(value: u64) -> transaction::Output {
		let keychain = keychain_for_tests();
		let key_id = keychain.derive_key_id(value as u32).unwrap();
		let commit = keychain.commit(value, &key_id).unwrap();
		let switch_commit = keychain.switch_commit(&key_id).unwrap();
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
		let msg = secp::pedersen::ProofMessage::empty();
		let proof = keychain.range_proof(value, &key_id, commit, msg).unwrap();

		transaction::Output {
			features: transaction::COINBASE_OUTPUT,
			commit: commit,
			switch_commit_hash: switch_commit_hash,
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
