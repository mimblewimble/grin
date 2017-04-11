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

//! Base types for the transaction pool's Directed Acyclic Graphs

use std::vec::Vec;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::Weak;
use std::cell::RefCell;
use std::collections::HashMap;

use secp::pedersen::Commitment;

use time;

use core::core;

/// An entry in the transaction pool.
/// These are the vertices of both of the graph structures
pub struct PoolEntry {
    // Core data
    // Unique identifier of this pool entry and the corresponding transaction
    pub transaction_hash: core::hash::Hash,

    // Metadata 
    size_estimate: u64,
    pub receive_ts: time::Tm,
}

impl PoolEntry {
    pub fn new(tx: &core::transaction::Transaction) -> PoolEntry {
        PoolEntry{
            transaction_hash: transaction_identifier(tx),
            size_estimate : estimate_transaction_size(tx),
            receive_ts: time::now()} 
    }
}

fn estimate_transaction_size(tx: &core::transaction::Transaction) -> u64 {
    0
}

/// An edge connecting graph vertices.
/// For various use cases, one of either the source or destination may be
/// unpopulated
pub struct Edge {
    // Source and Destination are the vertex id's, the transaction (kernel)
    // hash.
    source: Option<core::hash::Hash>,
    destination: Option<core::hash::Hash>,

    // Output is the output hash which this input/output pairing corresponds
    // to.
    output: Commitment,
}

impl Edge{
    pub fn new(source: Option<core::hash::Hash>, destination: Option<core::hash::Hash>, output: Commitment) -> Edge {
        Edge{source: source, destination: destination, output: output}
    }

    pub fn with_source(&self, src: Option<core::hash::Hash>) -> Edge {
        Edge{source: src, destination: self.destination, output: self.output}
    }

    pub fn with_destination(&self, dst: Option<core::hash::Hash>) -> Edge {
        Edge{source: self.source, destination: dst, output: self.output}
    }

    pub fn output_commitment(&self) -> Commitment {
        self.output
    }
    pub fn destination_hash(&self) -> Option<core::hash::Hash> {
        self.destination
    }
    pub fn source_hash(&self) -> Option<core::hash::Hash> {
        self.source
    }
}

/// The generic graph container. Both graphs, the pool and orphans, embed this
/// structure and add additional capability on top of it.
pub struct DirectedGraph {
    edges: HashMap<Commitment, Edge>,
    vertices: Vec<PoolEntry>,

    // A small optimization: keeping roots (vertices with in-degree 0) in a 
    // separate list makes topological sort a bit faster. (This is true for
    // Kahn's, not sure about other implementations)
    roots: Vec<PoolEntry>,
}

impl DirectedGraph {
    pub fn get_edge_by_commitment(&self, output_commitment: &Commitment) -> Option<&Edge> {
        self.edges.get(output_commitment)
    }

    pub fn remove_edge_by_commitment(&mut self, output_commitment: &Commitment) -> Option<Edge> {
        self.edges.remove(output_commitment)
    }

    pub fn remove_vertex(&mut self, tx_hash: core::hash::Hash) -> Option<PoolEntry> {
        match self.roots.iter().position(|x| x.transaction_hash == tx_hash) {
            Some(i) => Some(self.roots.swap_remove(i)),
            None => {
                match self.vertices.iter().position(|x| x.transaction_hash == tx_hash) {
                    Some(i) => Some(self.vertices.swap_remove(i)),
                    None => None,
                }
            }
        }
    }

    // add_entry is the all-in-one append method, which considers the vertex
    // destination (roots or vertices) and inserts the edges.
    // After calling add_entry, edges will be drained and can no longer be
    // used.
    pub fn add_entry(&mut self, vertex: PoolEntry, mut edges: Vec<Edge>) {
        if edges.len() == 0 {
            self.roots.push(vertex);
        } else {
            self.vertices.push(vertex);
            for edge in edges.drain(..) {
                self.edges.insert(edge.output_commitment(), edge);
            }
        }
    }

    // add_vertex_only adds a vertex, meant to be complemented by add_edge_only
    // in cases where delivering a vector of edges is not feasible or efficient
    pub fn add_vertex_only(&mut self, vertex: PoolEntry, is_root: bool) {
        if is_root {
            self.roots.push(vertex);
        } else {
            self.vertices.push(vertex);
        }
    }

    pub fn add_edge_only(&mut self, edge: Edge) {
        self.edges.insert(edge.output_commitment(), edge);
    }
}

/// The transaction identifier is not yet finalized. As implemented in
/// grin/core, it is the merkle root of a transaction; however this is not yet
/// exposed.
/// This method is a placeholder until a reasonable identifier is decided on.
pub fn transaction_identifier(tx: &core::transaction::Transaction) -> core::hash::Hash {
    unimplemented!();
}
