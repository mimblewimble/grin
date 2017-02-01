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
    pub tx_hash: core::hash::Hash,

    // Metadata 
    size_estimate: u64,
    pub receive_ts: time::Tm,
}

/// An edge connecting graph vertices.
/// For various use cases, one of either the source or destination may be
/// unpopulated
pub struct Edge {
    // Source and Destination are the vertex id's, the transaction hash.
    pub source: core::hash::Hash,
    pub destination: core::hash::Hash,

    // Output is the output hash which this input/output pairing corresponds
    // to.
    pub output: core::hash::Hash,
}

/// The generic graph container. Both graphs, the pool and orphans, embed this
/// structure and add additional capability on top of it.
pub struct DirectedGraph {
    edges: Vec<Edge>,
    vertices: Vec<PoolEntry>,

    // A small optimization: keeping roots (vertices with in-degree 0) in a 
    // separate list makes topological sort a bit faster. (This is true for
    // Kahn's, not sure about other implementations)
    roots: Vec<PoolEntry>,
}
