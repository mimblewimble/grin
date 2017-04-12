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

//! Base types for the transaction pool implementation.

use std::vec::Vec;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::cell::RefCell;
use std::collections::HashMap;

use secp::pedersen::Commitment;

pub use graph;
// temporary blockchain dummy impls
use blockchain::{DummyChain, DummyUtxoSet};

use time;

use core::core::transaction;
use core::core::block;
use core::core::hash;


/// Rough first pass: the data representing where we heard about a tx from.
pub struct TxSource {
    /// Human-readable name used for logging and errors.
    pub debug_name: String,
    /// Unique identifier used to distinguish this peer from others.
    pub identifier: String,
}

/// This enum describes the parent for a given input of a transaction.
#[derive(Clone)]
enum Parent {
    Unknown,
    BlockTransaction,
    PoolTransaction{tx_ref: hash::Hash},
    AlreadySpent{other_tx: hash::Hash},
}


enum PoolError {
    Invalid,
    AlreadyInPool,
    DuplicateOutput{other_tx: Option<hash::Hash>, in_chain: bool,
        output: Commitment},
    DoubleSpend{other_tx: hash::Hash, spent_output: Commitment},
    // An orphan successfully added to the orphans set
    OrphanTransaction,
}



/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
struct TransactionPool<'a> {
    pub transactions: HashMap<hash::Hash, Box<transaction::Transaction>>,

    pub pool : Pool,
    pub orphans: Orphans,

    // blockchain is a DummyChain, for now, which mimics what the future
    // chain will offer to the pool
    blockchain: &'a DummyChain,
}

/// Pool contains the elements of the graph that are connected, in full, to
/// the blockchain.
/// Reservations of outputs by orphan transactions (not fully connected) are
/// not respected.
/// Spending references (input -> output) exist in two structures: internal 
/// graph references are contained in the pool edge sets, while references 
/// sourced from the blockchain's UTXO set are contained in the 
/// blockchain_connections set.
/// Spent by references (output-> input) exist in two structures: pool-pool
/// connections are in the pool edge set, while unspent (dangling) references
/// exist in the available_outputs set.
struct Pool {
    graph : graph::DirectedGraph,

    // available_outputs are unspent outputs of the current pool set, 
    // maintained as edges with empty destinations, keyed by the 
    // output's hash.
    available_outputs: HashMap<Commitment, graph::Edge>,

    // Consumed blockchain utxo's are kept in a separate map. 
    consumed_blockchain_outputs: HashMap<Commitment, graph::Edge>
}

impl Pool {
    fn has_available_output(&self, c: &Commitment) -> bool {
        self.available_outputs.contains_key(c)
    }

    /// Given an output, return the transaction hash generating the 
    /// available (unspent) output commitment, if one exists.
    fn search_for_available_output(&self, c: &Commitment) -> Option<hash::Hash> {
        match self.available_outputs.get(c) {
            Some(e) => e.source_hash(),
            None => None
        }
    }

    /// Given an output, check if a spending reference (input -> output)
    /// already exists in the pool.
    /// Returns the transaction (kernel) hash corresponding to the conflicting
    /// transaction
    fn check_double_spend(&self, o: &transaction::Output) -> Option<hash::Hash> {
        self.graph.get_edge_by_commitment(&o.commitment()).or(self.consumed_blockchain_outputs.get(&o.commitment())).map(|x| x.destination_hash().unwrap())
    }

}

/// Orphans contains the elements of the transaction graph that have not been
/// connected in full to the blockchain. 
struct Orphans {
    graph : graph::DirectedGraph,

    // available_outputs are unspent outputs of the current orphan set, 
    // maintained as edges with empty destinations.
    available_outputs: HashMap<Commitment, graph::Edge>,

    // missing_outputs are spending references (inputs) with missing 
    // corresponding outputs, maintained as edges with empty sources.
    missing_outputs: HashMap<Commitment, graph::Edge>,

    // pool_connections are bidirectional edges which connect to the pool
    // graph. They should map one-to-one to pool graph available_outputs. 
    // pool_connections should not be viewed authoritatively, they are 
    // merely informational until the transaction is officially connected to
    // the pool.
    pool_connections: HashMap<Commitment, graph::Edge>,
}

impl Orphans {
    /// Checks for a double spent output, given the hash of the output, 
    /// ONLY in the data maintained by the orphans set. This includes links
    /// to the pool as well as links internal to orphan transactions.
    /// Returns the transaction hash corresponding to the conflicting
    /// transaction.
    fn check_double_spend(&self, o: transaction::Output) -> Option<hash::Hash> {
        self.graph.get_edge_by_commitment(&o.commitment()).or(self.pool_connections.get(&o.commitment())).map(|x| x.destination_hash().unwrap())
    }
}



impl<'a> TransactionPool<'a> {
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
        self.pool.search_for_available_output(output_commitment).
            map(|x| Parent::PoolTransaction{tx_ref: x}).
            or(self.search_blockchain_unspents(output_commitment)).
            or(self.search_pool_spents(output_commitment)).
            unwrap_or(Parent::Unknown)
    }

    // search_blockchain_unspents searches the current view of the blockchain
    // unspent set, represented by blockchain unspents - pool spents, for an
    // output designated by output_commitment.
    fn search_blockchain_unspents(&self, output_commitment: &Commitment) -> Option<Parent> {
        self.blockchain.get_best_utxo_set().get_output(output_commitment).
            map(|o| match self.pool.consumed_blockchain_outputs.get(output_commitment) {
                Some(x) => Parent::AlreadySpent{other_tx: x.destination_hash().unwrap()},
                None => Parent::BlockTransaction,
            })
    }

    // search_pool_spents is the second half of pool input detection, after the
    // available_outputs have been checked. This returns either a
    // Parent::AlreadySpent or None.
    fn search_pool_spents(&self, output_commitment: &Commitment) -> Option<Parent> {
        self.pool.graph.get_edge_by_commitment(output_commitment).
            map(|x| Parent::AlreadySpent{other_tx: x.destination_hash().unwrap()})

    }

    /// Attempts to add a transaction to the pool.
    ///
    /// Adds a transation to the memory pool, deferring to the orphans pool
    /// if necessary, and performing any connection-related validity checks.
    /// Happens under an exclusive mutable reference gated by the write portion
    /// of a RWLock.
    ///
    pub fn add_to_memory_pool(&mut self, source: TxSource, tx: transaction::Transaction) -> Result<(), PoolError> {
        // The first check invovles ensuring that an identical transaction is 
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
            return Err(PoolError::AlreadyInPool)
        }
        

        // The next issue is to identify all unspent outputs that
        // this transaction will consume and make sure they exist in the set.
        let mut pool_refs: Vec<graph::Edge> = Vec::new();
        let mut orphan_refs: Vec<graph::Edge> = Vec::new();
        let mut blockchain_refs: Vec<graph::Edge> = Vec::new();


        for input in &tx.inputs {
            let base = graph::Edge::new(None, Some(tx_hash), 
                input.commitment());

            match self.search_for_best_output(&input.commitment()) {
                Parent::PoolTransaction{tx_ref: x} => pool_refs.push(base.with_source(Some(x))),
                Parent::BlockTransaction => blockchain_refs.push(base),
                Parent::Unknown => orphan_refs.push(base),
                Parent::AlreadySpent{other_tx: x} => return Err(PoolError::DoubleSpend{other_tx: x, spent_output: input.commitment()}),
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
            // Checking against current blockchain unspent outputs
            // We want outputs even if they're spent by pool txs, so we ignore
            // consumed_blockchain_outputs
            if self.blockchain.get_best_utxo_set().get_output(&output.commitment()).is_some() {
                return Err(PoolError::DuplicateOutput{
                    other_tx: None,
                    in_chain: true,
                    output: output.commitment()})
            }


            // Check for generation of duplicate unspent outputs in the pool
            match self.pool.available_outputs.get(&output.commitment()) {
                Some(x) => {
                    return Err(PoolError::DuplicateOutput{
                    other_tx: x.source_hash(),
                    in_chain: false,
                    output: output.commitment()})
                    },
                None => {},
            }

            // Checking the spent references for duplicate outputs.
            match self.pool.graph.get_edge_by_commitment(&output.commitment()) {
                Some(x) => {
                return Err(PoolError::DuplicateOutput{
                    other_tx: x.source_hash(),
                    in_chain: false,
                    output: output.commitment()})
                }
                None => {},

            }

            // If the transaction might go into orphans, perform the same 
            // checks as above but against the orphan set instead.
            if is_orphan {
                // Checking against new unspents orphans generate
                match self.orphans.available_outputs.get(&output.commitment()){
                    Some(x) => {
                    return Err(PoolError::DuplicateOutput{
                        other_tx: x.source_hash(),
                        in_chain: false,
                        output: output.commitment()})
                    },
                    None => {},
                }

                // Checking against spent refs within the orphans graph
                match self.orphans.graph.get_edge_by_commitment(&output.commitment()){
                    Some(x) => {
                    return Err(PoolError::DuplicateOutput{
                        other_tx: x.source_hash(),
                        in_chain: false,
                        output: output.commitment()})
                    },
                    None => {},
                }

                // No need to check pool connections since those are covered
                // by pool unspents.
            }
        }

        // Assertion: we have exactly as many resolved spending references as
        // inputs to the transaction.
        assert_eq!(tx.inputs.len(), 
            blockchain_refs.len() + pool_refs.len() + orphan_refs.len());

        // At this point we know if we're spending all known unspents and not
        // creating any duplicate unspents.
        let pool_entry = graph::PoolEntry::new(&tx);

        if !is_orphan {
            // In the non-orphan (pool) case, we've ensured that every input
            // maps one-to-one with an unspent (available) output, and each
            // output is unique. No further checks are necessary.

            // Removing consumed available_outputs
            for new_edge in &pool_refs {
                self.pool.available_outputs.remove(&new_edge.output_commitment());
            }

            // Accounting for consumed blockchain outputs
            for new_blockchain_edge in blockchain_refs.drain(..) {
                self.pool.consumed_blockchain_outputs.insert(
                    new_blockchain_edge.output_commitment(),
                    new_blockchain_edge);
            }

            // Adding the transaction to the vertices list along with internal
            // pool edges
            self.pool.graph.add_entry(pool_entry, pool_refs);

            // Adding the new unspents to the unspent map
            for unspent_output in tx.outputs.iter()
                .map(|x| graph::Edge::new(Some(tx_hash), None, x.commitment())) {

                self.pool.available_outputs.insert(unspent_output.output_commitment(),
                    unspent_output);
            }

            self.reconcile_orphans();
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
            for pool_ref in &pool_refs {
                match self.orphans.pool_connections.get(&pool_ref.output_commitment()){
                    // Should the below err be subtyped to orphans somehow? 
                    Some(x) => return Err(PoolError::DoubleSpend{other_tx: x.destination_hash().unwrap(), spent_output: x.output_commitment()}),
                    None => {},
                }
            }

            // Next, we have to consider the possibility of double spends
            // within the orphans set.
            // We also have to distinguish now between missing and internal
            // references.
            let mut missing_refs: HashMap<usize, ()> = HashMap::new();
            for (i, orphan_ref) in orphan_refs.iter().enumerate() {
                // If the input is in orphans available_outputs, everything is
                // good.
                if !self.orphans.available_outputs.contains_key(&orphan_ref.output_commitment()) {

                    // Otherwise, we have to check for spends within the orphan
                    // set (pool and blockchain connections are already 
                    // resolved), and duplicate missing_outputs
                    match self.orphans.graph.get_edge_by_commitment(&orphan_ref.output_commitment()) {
                        Some(x) => return Err(PoolError::DoubleSpend{
                            other_tx: x.destination_hash().unwrap(),
                            spent_output: x.output_commitment()}),
                        None => {
                            // The reference does not resolve to anything.
                            // Make sure this missing_output has not already
                            // been claimed, then add this entry to 
                            // missing_refs
                            match self.orphans.missing_outputs.get(&orphan_ref.output_commitment()) {
                                Some(x) => return Err(PoolError::DoubleSpend{
                                    other_tx: x.destination_hash().unwrap(),
                                    spent_output: x.output_commitment()}),
                                None => missing_refs.insert(i, ()),
                            };
                        },
                    }
                }
            }

            // We have passed all failure modes.
            // Add pool_refs
            for pool_ref in pool_refs.drain(..).chain(blockchain_refs.drain(..)) {
                self.orphans.pool_connections.insert(
                    pool_ref.output_commitment(), pool_ref);
            }

            // if missing_refs is the same length as orphan_refs, we have
            // no orphan-orphan links for this transaction and it is a 
            // root transaction of the orphans set
            self.orphans.graph.add_vertex_only(pool_entry,
                missing_refs.len() == orphan_refs.len());

            for (i, orphan_ref) in orphan_refs.drain(..).enumerate() {
                if missing_refs.contains_key(&i) {
                    self.orphans.missing_outputs.insert(
                        orphan_ref.output_commitment(),
                        orphan_ref);
                } else {
                    self.orphans.graph.add_edge_only(orphan_ref);
                }
            }

            Err(PoolError::OrphanTransaction)
        }
        
    }

    /// The primary goal of the reconcile_orphans method is to eliminate any 
    /// orphans who conflict with the recently accepted pool transaction.
    /// TODO: How do we handle fishing orphans out that look like they could
    /// be freed? Current thought it to do so under a different lock domain
    /// so that we don't have the potential for long recursion under the write
    /// lock.
    pub fn reconcile_orphans(&self)-> Result<(),PoolError> {
        unimplemented!()
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
    pub fn reconcile_block(&mut self, block: &block::Block) -> Result<Vec<Box<transaction::Transaction>>, PoolError> {
        // Prepare the new blockchain-only UTXO view for this process
        let updated_blockchain_utxo =
            self.blockchain.get_best_utxo_set().apply(block);

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
        let mut marked_transactions: HashMap<hash::Hash, ()> = HashMap::new();
        {
            let mut conflicting_edges: Vec<&graph::Edge> = block.inputs.iter().
                filter_map(|x| 
                   self.pool.consumed_blockchain_outputs.get(&x.commitment())).
                collect();

            let mut conflicting_outputs: Vec<&graph::Edge> = block.outputs.iter().
                filter_map(|x: &transaction::Output| 
                    self.pool.graph.get_edge_by_commitment(&x.commitment()).
                    or_else(|| self.pool.available_outputs.get(&x.commitment()))).
                collect();

            conflicting_edges.append(&mut conflicting_outputs);


            for edge in &conflicting_edges {
                self.mark_transaction(&updated_blockchain_utxo,
                    &edge, &mut marked_transactions);
            }
        }
        let freed_txs = self.sweep_transactions(marked_transactions,
            &updated_blockchain_utxo);

        self.reconcile_orphans();

        Ok(freed_txs)
    }

    /// The mark portion of our mark-and-sweep pool cleanup.
    ///
    /// The transaction designated as the recipient of the conflicting_edge is
    /// immediately marked. Each output of this transaction is then examined;
    /// if a transaction in the pool spends this output and the output is not
    /// replaced by an identical output included in the updated UTXO set, the
    /// child is marked as well and the process continues recursively.
    ///
    /// Marked transactions are added to the mutable marked_txs HashMap which
    /// is supplied by the calling function.
    fn mark_transaction(&self, updated_utxo: &DummyUtxoSet,
        conflicting_edge: &graph::Edge, 
        marked_txs: &mut HashMap<hash::Hash, ()>) {

        marked_txs.insert(conflicting_edge.destination_hash().unwrap(),
            ());

        let tx_ref = self.transactions.get(&conflicting_edge.destination_hash().unwrap());

        for output in &tx_ref.unwrap().outputs {
            match self.pool.graph.get_edge_by_commitment(&output.commitment()) {
                Some(x) => {
                    if updated_utxo.get_output(&x.output_commitment()).is_none() {
                        self.mark_transaction(updated_utxo, &x, marked_txs);
                    }
                },
                None => {},
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
    fn sweep_transactions(&mut self,
        marked_transactions: HashMap<hash::Hash, ()>,
        updated_utxo: &DummyUtxoSet)->Vec<Box<transaction::Transaction>> {

        let mut removed_txs = Vec::new();

        for tx_hash in marked_transactions.keys() {
            let removed_tx = self.transactions.remove(tx_hash).unwrap();

            // Input edge conditions:
            // 1. If the input edge is a blockchain connection, remove it.
            // 2. If the input edge connects to a deleted transaction,
            //      remove it.
            // 3. If the input edge connects to a non-deleted transaction,
            //      add the edge to the unspent set.
            //
            // Note that some of the edges in condition 2 may have been
            // removed by output edge removal if that transaction was 
            // visited first. As written, that will result in an attempt to
            // remove the edge from blockchain_connections, which should be
            // safe.
            for input in &removed_tx.inputs {
                match self.pool.graph.remove_edge_by_commitment(&input.commitment()) {
                    Some(x) => {
                        if !marked_transactions.contains_key(&x.source_hash().unwrap()) {
                            self.pool.available_outputs.insert(
                                x.output_commitment(), 
                                x.with_destination(None));
                        }
                    },
                    None => {
                        self.pool.consumed_blockchain_outputs.remove(
                            &input.commitment());
                    },
                };
            }

            // Output edge conditions: 
            // 1. If the output edge is an available_output, remove it.
            // 2. If the output edge leads to a deleted transaction, remove it.
            // 3. If the output edge leads to a non-deleted transaction, 
            //   replace it with a new blockchain_connection.
            //
            // As above, some outputs may be missing from condition 2 if the 
            // spending transaction was visited first. 
            for output in &removed_tx.outputs {
                match self.pool.graph.remove_edge_by_commitment(&output.commitment()) {
                    Some(x) => {
                        if !marked_transactions.contains_key(&x.destination_hash().unwrap()) {
                            self.pool.consumed_blockchain_outputs.insert(
                                x.output_commitment(),
                                x.with_source(None));
                        }
                    },
                    None => {
                        self.pool.available_outputs.remove(&output.commitment());
                    },
                };
            }

            removed_txs.push(removed_tx);
        }
        removed_txs
    }

}
