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

pub use graph;

use time;

use core::core;


/// Rough first pass: the data representing where we heard about a tx from.
pub struct TxSource {
    /// Human-readable name used for logging and errors.
    pub debug_name: String,
    /// Unique identifier used to distinguish this peer from others.
    pub identifier: String,
}

/*
/// This enum describes the parent for a given input of a transaction.
#[derive(Clone)]
enum Parent {
    Unknown,
    BlockTransaction{hash: core::hash::Hash},
    PoolTransaction{hash: core::hash::Hash, tx_ref: Weak<RefCell<PoolEntry>>},
    OrphanTransaction{hash: core::hash::Hash, tx_ref: Weak<RefCell<PoolEntry>>},
}

*/
enum PoolError {
    Invalid,
    DuplicateOutput{other_tx: core::hash::Hash, output: Commitment},
    DoubleSpend{other_tx: core::hash::Hash, spent_output: Commitment},
}



/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
struct TransactionPool {
    pub transactions: HashMap<core::hash::Hash, Box<core::transaction::Transaction>>,

    pub pool : Pool,
    pub orphans: Orphans,
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
    /// TODO: Once we have stability in the blockchain UTXO set, that should
    /// go here.
    fn search_for_output(&self, c: &Commitment) -> Option<core::hash::Hash> {
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
        self.graph.get_edge_by_id(o.commitment()).or(self.consumed_blockchain_outputs.get(o.commitment())).map(|x| x.output_commitment())
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
        self.graph.get_edge_by_id(o.commitment()).or(self.pool_connections.get(o.commitment())).map(|x| x.output_commitment())
    }
}



impl TransactionPool {
    /// Searches for an output, designated by its hash, from the current best
    /// UTXO view, presented by taking the best blockchain UTXO set (as
    /// determined by the blockchain component) and rectifying pool spent and
    /// unspents.
    /// Returns a bool determining if the output was found, and an Option 
    /// containing the transaction (kernel) hash of the transaction 
    /// generating the output, if the transaction is in the pool. (Once in the
    /// blockchain, transaction association is irreversibly lost.)
    pub fn search_for_best_output(&self, output_commitment: Commitment) -> (bool, Option<core::hash::Hash>) {
        // The current best unspent set is: 
        //   Pool unspent + (blockchain unspent - pool->blockchain spent)
        // Pool unspents are unconditional so we check those first
        self.available_outputs.get(output_commitment).map(|x| (true, x)).or_
            Some(x) => (true, x),
            None => self.o
        }
    }

    /// Add a transation to the memory pool, deferring to the orphans pool
    /// if necessary.
    /// Happens under an exclusive mutable reference gated by the write portion
    /// of a RWLock.
    pub fn add_to_memory_pool(&mut self, source: TxSource, tx: core::transaction::Transaction) -> Result<(), PoolError> {
        // The first issue is to identify all unspent outputs that
        // this transaction will consume and make sure they exist in the set.
        let mut pool_refs: Vec<graph::Edge> = Vec::new();
        let mut orphan_refs: Vec<graph::Edge> = Vec::new();
        let mut blockchain_refs: Vec<graph::Edge> = Vec::new();

        let tx_hash = tx.hash();

        for input in tx.inputs.iter() {
            //TODO: Check the blockchain data source alongside the pool
            match self.pool.search_for_output(&input.commitment()) {
                Some(x) => pool_refs.push(x.with_destination(tx_hash)),
                None => orphan_refs.push(
                    graph::Edge::new(None, Some(tx_hash), input.commitment())),
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
            //TODO: Check the blockchain data source

            // Check for generation of duplicate unspent outputs
            if self.pool.available_outputs.contains_key(&output.commitment()) {
                return Err(PoolError::DuplicateOutput{
                    other_tx: self.pool.available_outputs.get(
                      &output.commitment()).unwrap().source_hash(),
                    output: &output.commitment()})
            }

            // Checking the spent references for duplicate outputs.
            if self.pool.graph.edges.contains_key(&output.commitment()) {
                return Err(PoolError::DuplicateOutput{
                    other_tx: self.pool.graph.edges
                        .get(&output.commitment()).unwrap().source_hash(),
                    output: &output.commitment()})

            }

            // If the transaction might go into orphans, perform the same 
            // checks as above but against the orphan set instead.
            if is_orphan {
                if self.orphans.available_outputs.contains_key(&output.commitment()){
                    return Err(PoolError::DuplicateOutput{
                        other_tx: self.orphans.available_outputs.get(
                            &output.commitment()).unwrap().source_hash(),
                        output: &output.commitment()})
                }

                if self.orphans.graph.edges.contains_key(&output.output_hash()) {
                    return Err(PoolError::DuplicateOutput{
                        other_tx: self.orphans.graph.edges.get(
                            &output.commitment()).unwrap().source_hash(),
                        output: &output.commitment()})
                }
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

            // Adding the transaction to the vertices list
            self.pool.graph.vertices.push(pool_entry);

            // Adding the consumed inputs to the edges list
            for new_edge in pool_refs.iter() {
                self.pool.available_outputs.remove(new_edge.output_commitment());
                self.pool.graph.edges.insert(new_edge.output_commitment(),
                    new_edge);
            }

            // Adding the new unspents to the unspent map
            for unspent_output in tx.outputs.iter()
                .map(|x| graph::Edge::new(tx_hash, None, x.hash())) {

                self.available_outputs.insert(unspent_output.output_commitment(),
                    unspent_output);
            }

            self.resolve_orphans(tx)
        } else {
            // is_orphan is a bit of a misnomer: We don't know for sure that
            // the transaction is an orphan yet. What we do know is that it's
            // not an acceptable pool transaction, because one or more of its
            // input references does not resolve to the pool's unspent output
            // set.
            
            // We have one remaining failure condition, which is the double
            // spend case.
            // We know that all inputs spending unclaimed unspents in the 
            // pool are accounted for. The remainder, contained
            // within the orphan_refs vector, need to be checked.
            for orphan_ref in orphan_refs.iter() {
            }
            
            self.orphans.graph.vertices.push(pool_entry);
            Ok(())
        }
    }

    /// Resolve orphans to a given transaction: fish out any transactions
    /// whose unresolved links have been satisfied by the addition of the
    /// input transaction.
    pub fn resolve_orphans(&self, tx: core::transaction::Transaction) -> Result<(),PoolError> {
        unimplemented!();
    }
}
