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

//! The primary module containing the implementations of the transaction pool
//! and its top-level members.

use std::vec::Vec;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use secp::pedersen::Commitment;

pub use graph;

use time;

use core::core::transaction;
use core::core::block;
use core::core::hash;



/// Placeholder: the data representing where we heard about a tx from.
///
/// Used to make decisions based on transaction acceptance priority from 
/// various sources. For example, a node may want to bypass pool size
/// restrictions when accepting a transaction from a local wallet.
///
/// Most likely this will evolve to contain some sort of network identifier, 
/// once we get a better sense of what transaction building might look like.
pub struct TxSource {
    /// Human-readable name used for logging and errors.
    pub debug_name: String,
    /// Unique identifier used to distinguish this peer from others.
    pub identifier: String,
}

/// This enum describes the parent for a given input of a transaction.
#[derive(Clone)]
pub enum Parent {
    Unknown,
    BlockTransaction,
    PoolTransaction{tx_ref: hash::Hash},
    AlreadySpent{other_tx: hash::Hash},
}

impl fmt::Debug for Parent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &Parent::Unknown => write!(f, "Parent: Unknown"),
            &Parent::BlockTransaction => write!(f, "Parent: Block Transaction"),
            &Parent::PoolTransaction{tx_ref: x} => write!(f,
                "Parent: Pool Transaction ({:?})", x),
            &Parent::AlreadySpent{other_tx: x} => write!(f,
                "Parent: Already Spent By {:?}", x),
        }
    }
}

#[derive(Debug)]
pub enum PoolError {
    Invalid,
    AlreadyInPool,
    DuplicateOutput{other_tx: Option<hash::Hash>, in_chain: bool,
        output: Commitment},
    DoubleSpend{other_tx: hash::Hash, spent_output: Commitment},
    // An orphan successfully added to the orphans set
    OrphanTransaction,
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
pub struct Pool {
    graph : graph::DirectedGraph,

    // available_outputs are unspent outputs of the current pool set, 
    // maintained as edges with empty destinations, keyed by the 
    // output's hash.
    available_outputs: HashMap<Commitment, graph::Edge>,

    // Consumed blockchain utxo's are kept in a separate map. 
    consumed_blockchain_outputs: HashMap<Commitment, graph::Edge>
}

impl Pool {
    pub fn has_available_output(&self, c: &Commitment) -> bool {
        self.available_outputs.contains_key(c)
    }

    /// Given an output, return the transaction hash generating the 
    /// available (unspent) output commitment, if one exists.
    pub fn search_for_available_output(&self, c: &Commitment) -> Option<hash::Hash> {
        match self.available_outputs.get(c) {
            Some(e) => e.source_hash(),
            None => None
        }
    }

    /// Given an output, check if a spending reference (input -> output)
    /// already exists in the pool.
    /// Returns the transaction (kernel) hash corresponding to the conflicting
    /// transaction
    pub fn check_double_spend(&self, o: &transaction::Output) -> Option<hash::Hash> {
        self.graph.get_edge_by_commitment(&o.commitment()).or(self.consumed_blockchain_outputs.get(&o.commitment())).map(|x| x.destination_hash().unwrap())
    }


    pub fn get_blockchain_spent(&self, c: &Commitment) -> Option<&graph::Edge> {
        self.consumed_blockchain_outputs.get(c)
    }

    pub fn get_internal_spent(&self, c: &Commitment) -> Option<&graph::Edge> {
        self.graph.get_edge_by_commitment(c)
    }

}

impl TransactionGraphContainer for Pool { 
    fn get_graph(&self) -> &graph::DirectedGraph {
        &self.graph
    }
    fn get_available_outputs(&self) -> &HashMap<Commitment, graph::Edge> {
        &self.available_outputs
    }
}

/// Orphans contains the elements of the transaction graph that have not been
/// connected in full to the blockchain. 
pub struct Orphans {
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

impl TransactionGraphContainer for Orphans {
    fn get_graph(&self) -> &graph::DirectedGraph {
        &self.graph
    }
    fn get_available_outputs(&self) -> &HashMap<Commitment, graph::Edge> {
        &self.available_outputs
    }
}

/// Trait for types that combine a graph with available_outputs
pub trait TransactionGraphContainer {
    fn get_graph(&self) -> &graph::DirectedGraph;
    fn get_available_outputs(&self) -> &HashMap<Commitment, graph::Edge>;

    fn has_available_output(&self, c: &Commitment) -> bool {
        self.get_available_outputs().contains_key(c)
    }
    /// Checks if the pool has anything by this output already, between 
    /// available outputs and internal ones.
    fn find_output(&self, c: &Commitment) -> Option<hash::Hash> {
        self.get_available_outputs().get(c).
            or(self.get_graph().get_edge_by_commitment(c)).
            map(|x| x.source_hash().unwrap())
    }
}
