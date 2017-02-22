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
    Orphan{missing_hash: core::hash::Hash},
    DuplicateOutput{other_tx: core::hash::Hash},
}



/// The pool itself.
/// The transactions HashMap holds ownership of all transactions in the pool,
/// keyed by their transaction hash.
struct TransactionPool {
    pub transactions: HashMap<core::hash::Hash, Box<core::transaction::Transaction>>,

    pub pool : RwLock<Pool>,
    pub orphans: RwLock<Orphans>,
}

/// Pool contains the elements of the graph that are connected, in full, to
/// the blockchain.
/// Reservations of outputs by orphan transactions (not fully connected) are
/// not respected.
struct Pool {
    graph : graph::DirectedGraph,

    // available_outputs are unspent outputs of the current pool set, 
    // maintained as edges with empty destinations, keyed by the 
    // output's hash.
    available_outputs: HashMap<core::hash::Hash, graph::Edge>,
}

impl Pool {
    fn search_for_output(&self, h: & core::hash::Hash) -> bool {
        self.available_outputs.contains_key(h)
    }

    // This happens either under a lock or an exclusive borrowed mutable ref,
    // so no risk of a race causing double-allocation or deallocation.
    fn connect_transaction(&mut self, tx: &core::transaction::Transaction) -> Result<(), PoolError> {
        // The expectation is a cheap and parallelizable call to 
        // search_for_output or a similar non-authoritative check will gate
        // calls to connect_transaction, which can be a bit more expensive.
        
        // The first issue is to identify all unspent outputs that
        // this transaction will consume and make sure they exist in the set.
        for input in tx.inputs.iter() {
            //TODO: Check the blockchain data source
            if !self.available_outputs.contains_key(&input.output_hash()) {
                return Err(PoolError::Orphan{
                    missing_hash: input.output_hash()})
            }
        }

        // Next we examine the outputs this transaction creates and ensure
        // that they do not already exist.
        for output in tx.outputs.iter() {
            //TODO: Check the blockchain data source
            // An interesting note: if the blockchain creates an UTXO that's
            // a duplicate of this, its not always an issue.
            // If that output is spent by another transaction it is possible
            // for that transaction and this one to coexist. However, it does
            // impose some nontrivial ordering constraints: the transaction
            // spending the duplicate blockchain UTXO MUST be confirmed before
            // this one. In the interest of simplicity in this first pass we
            // will reject transactions which have unspents that are duplicates
            // of the ones in the blockchain.
            if self.available_outputs.contains_key(&output.hash()) {
                return Err(PoolError::DuplicateOutput{
                    other_tx: self.available_outputs.get(&output.output_hash()).unwrap().destination_hash()})
            }
            // TODO: Do we need to investigate for duplicate spents?
            // I believe the answer is no: Edges are tightly bound to tx
            // hashes for source and destination, so we should be able
            // to resolve these unambigously.
            // However, reusing outputs does lead to a number of really 
            // unpleasant situations, including potential replay attacks.
            // It probably makes sense to discourage this as much as 
            // possible.
        }

        // At this point we know we're spending all known unspents and not
        // creating any duplicate unspents.
        let pool_entry = graph::PoolEntry::new(&tx);
        // Adding the transaction to the vertices list
        self.graph.vertices.push(pool_entry);

        let tx_hash = tx.hash();

        // Adding the consumed inputs to the edges list
        for new_edge in tx.inputs.iter()
            .map(|x| self.available_outputs.remove(&x.output_hash()))
            .unwrap()
            .map(|x| x.with_destination(tx_hash)) {

                self.graph.edges.push(new_edge);
        }

        // Adding the new unspents to the unspent map
        for unspent_output in tx.outputs.iter()
            .map(|x| graph::Edge::new(tx_hash, None, x.hash())) {

            self.available_outputs.insert(unspent_output.output_hash(),
                unspent_output);
        }

        Ok(())
    }

    fn rollback_transaction(&mut self, removed_outputs: Vec<graph::Edge>, 
        added_outputs: Option<Vec<graph::Edge>>) {

        for replace_out in removed_outputs.drain(..) {
            self.available_outputs.insert(replace_out.output, replace_out);
        }

        for remove_out in added_outputs.unwrap_or(Vec::new()).drain(..) {
            self.available_outputs.remove(remove_out.output)
        }
    }

}

/// Orphans contains the elements of the transaction graph that have not been
/// connected in full to the blockchain. 
struct Orphans {
    graph : graph::DirectedGraph,

    // available_outputs are unspent outputs of the current orphan set, 
    // maintained as edges with empty destinations.
    available_outputs: Vec<graph::Edge>,

    // missing_outputs are spending references (inputs) with missing 
    // corresponding outputs, maintained as edges with empty sources.
    missing_outputs: Vec<graph::Edge>,

    // pool_connections are bidirectional edges which connect to the pool
    // graph. They should map one-to-one to pool graph available_outputs. 
    // pool_connections should not be viewed authoritatively, they are 
    // merely informational until the transaction is officially connected to
    // the pool.
    pool_connections: Vec<graph::Edge>,
}



impl TransactionPool {
    pub fn add_to_memory_pool(&self, source: TxSource, tx: core::transaction::Transaction) -> Result<(), PoolError> {
        // Placeholder: validation
        //tx.verify_sig;

        // Find the parent transactions
        // First, from the blockchain
        //
        // Next, from the pool
        
        // Now if it looks OK, take the lock and connect
        // TODO: Handle the poison case
        match self.pool.write().unwrap().connect_transaction(tx) {
            Ok(_) => return Ok(()),
            Err(e) => Err(e),
        }
            
        Ok(())
    }
}

