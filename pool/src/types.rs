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
use blockchain::DummyChain;

use time;

use core::core;


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
    PoolTransaction{tx_ref: core::hash::Hash},
    AlreadySpent{other_tx: core::hash::Hash},
}


enum PoolError {
    Invalid,
    AlreadyInPool,
    DuplicateOutput{other_tx: Option<core::hash::Hash>, in_chain: bool,
        output: Commitment},
    DoubleSpend{other_tx: core::hash::Hash, spent_output: Commitment},
    // An orphan successfully added to the orphans set
    OrphanTransaction,
}



/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
struct TransactionPool<'a> {
    pub transactions: HashMap<core::hash::Hash, Box<core::transaction::Transaction>>,

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
    fn search_for_available_output(&self, c: &Commitment) -> Option<core::hash::Hash> {
        match self.available_outputs.get(c) {
            Some(e) => e.source_hash(),
            None => None
        }
    }

    /// Given an output, check if a spending reference (input -> output)
    /// already exists in the pool.
    /// Returns the transaction (kernel) hash corresponding to the conflicting
    /// transaction
    fn check_double_spend(&self, o: &core::transaction::Output) -> Option<core::hash::Hash> {
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
    fn check_double_spend(&self, o: core::transaction::Output) -> Option<core::hash::Hash> {
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

    /// Add a transation to the memory pool, deferring to the orphans pool
    /// if necessary.
    /// Happens under an exclusive mutable reference gated by the write portion
    /// of a RWLock.
    pub fn add_to_memory_pool(&mut self, source: TxSource, tx: core::transaction::Transaction) -> Result<(), PoolError> {
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


        for input in tx.inputs.iter() {
            let base = graph::Edge::new(None, Some(tx_hash), 
                input.commitment());

            match self.search_for_best_output(&input.commitment()) {
                Parent::PoolTransaction{tx_ref: x} => pool_refs.push(base.with_source(x)),
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
        for output in tx.outputs.iter() {
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
            for new_edge in pool_refs.iter() {
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

            self.reconcile_orphans(&tx);
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
            for pool_ref in pool_refs.iter() {
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
    pub fn reconcile_orphans(&self, tx: &core::transaction::Transaction) -> Result<(),PoolError> {
        unimplemented!()
    }
}
