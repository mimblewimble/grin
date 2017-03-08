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

use time;

use core::core;

/// An entry in the transaction pool.
/// These are the vertices of both of the graph structures
pub struct PoolEntry {
    // Core data
    // Unique identifier of this pool entry and the corresponding transaction
    // (kernel hash?)
    pub kernel_hash: core::hash::Hash,

    // Metadata 
    size_estimate: u64,
    pub receive_ts: time::Tm,
}

impl PoolEntry {
    pub fn new(tx: &core::transaction::Transaction) -> PoolEntry {
        PoolEntry{
            tx_hash: tx.hash(),
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
    output: core::hash::Hash,
}

impl Edge{
    pub fn new(source: Option<core::hash::Hash>, destination: Option<core::hash::Hash>, output_hash: core::hash::Hash) -> Edge {
        Edge{source: source, destination: destination, output: output_hash}
    }

    pub fn with_source(&self, src: core::hash::Hash) -> Edge {
        Edge{source: Some(src), destination: self.destination, output: self.output}
    }

    pub fn with_destination(&self, dst: core::hash::Hash) -> Edge {
        Edge{source: self.source, destination: Some(dst), output: self.output}
    }

    pub fn output_hash(&self) -> core::hash::Hash {
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
    edges: HashMap<core::hash::Hash, Edge>,
    vertices: Vec<PoolEntry>,

    // A small optimization: keeping roots (vertices with in-degree 0) in a 
    // separate list makes topological sort a bit faster. (This is true for
    // Kahn's, not sure about other implementations)
    roots: Vec<PoolEntry>,
}
