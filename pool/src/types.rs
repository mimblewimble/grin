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


/// Rough first pass: the trait representing where we heard about a tx from.
pub trait TxSource {
    /// Human-readable name used for logging and errors.
    fn debug_name(&self) -> &str;
    /// Unique identifier used to distinguish this peer from others.
    fn identifier(&self) -> &str;
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
    AlreadySpent{other_tx: core::hash::Hash},
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
    fn search_for_output(&self, h: core::hash::Hash) -> bool {
        self.available_outputs.contains_key(h)
    }

    // This happens either under a lock or an exclusive borrowed mutable ref,
    // so no risk of a race causing double-allocation or deallocation.
    fn connect_transaction(&mut self, tx: core::transaction::Transaction) -> Result<(), PoolError> {
        // The expectation is a cheap and parallelizable call to 
        // search_for_output or a similar non-authoritative check will gate
        // calls to connect_transaction, which can be a bit more expensive.
        
        // The first issue is to identify and reserve all unspent outputs that
        // this transaction will consume.

        // We want to do this in one iter but with rollback capability if a
        // needed output is already spent, so this vector holds the outputs
        // we remove from the available map. 
        let removed_outputs = Vec::new();
        for input in tx.inputs.iter() {
            match self.available_outputs.remove(input.output_hash()) {
                Some(x) => removed_outputs.push(x),
                None => {
                    for replace_out in removed_outputs.iter() {
                        self.available_outputs.insert(replace_out.Output, replace_out);
                    }
                    return Err(PoolError::Orphan{missing_hash: input.output_hash()})
                },
            }
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
        // Using unwrap here: the only possible error is a poisonError, which
        // we don't have a good recovery for.
        // If this becomes an issue, we can rebuild the map from the graph
        // representations.
        let output_map = self.by_output.read().unwrap();
        let parents = vec![Parent::Unknown; tx.inputs.len()]; 
        for (i, input) in tx.inputs.iter().enumerate() {
            // First, check the confirmed UTXO state.
            
            // Next, check against pool state.
            
        }
        Ok(())
    }
}

fn parent_from_weak_ref(h: core::hash::Hash, p: &Weak<RefCell<graph::PoolEntry>>) -> Parent {
    p.upgrade().and_then(|x| parent_from_tx_ref(h, x)).unwrap_or(Parent::Unknown)
}

fn parent_from_tx_ref(h: core::hash::Hash, tx_ref: Arc<RefCell<PoolEntry>>) -> Parent {
    if tx_ref.borrow().is_orphaned() {
        return Parent::OrphanTransaction{hash: h, tx_ref: tx_ref.downgrade()};
    }
    Parent::PoolTransaction{hash: h, tx_ref: tx_ref}
}
