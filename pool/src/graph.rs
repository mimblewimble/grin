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
use std::collections::{HashMap, HashSet};

use util::secp::pedersen::Commitment;

use time;

use std::fmt;

use core::core;
use core::core::hash::Hashed;

/// An entry in the transaction pool.
/// These are the vertices of both of the graph structures
#[derive(Debug, PartialEq, Clone)]
pub struct PoolEntry {
	// Core data
	/// Unique identifier of this pool entry and the corresponding transaction
	pub transaction_hash: core::hash::Hash,

	// Metadata
	/// Size estimate
	pub size_estimate: u64,
	/// Receive timestamp
	pub receive_ts: time::Tm,
}

impl PoolEntry {
	/// Create new transaction pool entry
	pub fn new(tx: &core::transaction::Transaction) -> PoolEntry {
		PoolEntry {
			transaction_hash: transaction_identifier(tx),
			size_estimate: estimate_transaction_size(tx),
			receive_ts: time::now_utc(),
		}
	}
}

/// TODO guessing this needs implementing
fn estimate_transaction_size(_tx: &core::transaction::Transaction) -> u64 {
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

impl Edge {
	/// Create new edge
	pub fn new(
		source: Option<core::hash::Hash>,
		destination: Option<core::hash::Hash>,
		output: Commitment,
	) -> Edge {
		Edge {
			source: source,
			destination: destination,
			output: output,
		}
	}

	/// Create new edge with a source
	pub fn with_source(&self, src: Option<core::hash::Hash>) -> Edge {
		Edge {
			source: src,
			destination: self.destination,
			output: self.output,
		}
	}

	/// Create new edge with destination
	pub fn with_destination(&self, dst: Option<core::hash::Hash>) -> Edge {
		Edge {
			source: self.source,
			destination: dst,
			output: self.output,
		}
	}

	/// The output commitment of the edge
	pub fn output_commitment(&self) -> Commitment {
		self.output
	}

	/// The destination hash of the edge
	pub fn destination_hash(&self) -> Option<core::hash::Hash> {
		self.destination
	}

	/// The source hash of the edge
	pub fn source_hash(&self) -> Option<core::hash::Hash> {
		self.source
	}
}

impl fmt::Debug for Edge {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(
			f,
			"Edge {{source: {:?}, destination: {:?}, commitment: {:?}}}",
			self.source,
			self.destination,
			self.output
		)
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
	/// Create an empty directed graph
	pub fn empty() -> DirectedGraph {
		DirectedGraph {
			edges: HashMap::new(),
			vertices: Vec::new(),
			roots: Vec::new(),
		}
	}

	/// Get an edge by its commitment
	pub fn get_edge_by_commitment(&self, output_commitment: &Commitment) -> Option<&Edge> {
		self.edges.get(output_commitment)
	}

	/// Remove an edge by its commitment
	pub fn remove_edge_by_commitment(&mut self, output_commitment: &Commitment) -> Option<Edge> {
		self.edges.remove(output_commitment)
	}

	/// Remove a vertex by its hash
	pub fn remove_vertex(&mut self, tx_hash: core::hash::Hash) -> Option<PoolEntry> {
		match self.roots
			.iter()
			.position(|x| x.transaction_hash == tx_hash)
		{
			Some(i) => Some(self.roots.swap_remove(i)),
			None => match self.vertices
				.iter()
				.position(|x| x.transaction_hash == tx_hash)
			{
				Some(i) => Some(self.vertices.swap_remove(i)),
				None => None,
			},
		}
	}

	/// Promote any non-root vertices to roots based on current edges.
	/// For a given tx, if there are no edges with that tx as destination then
	/// it is a root.
	pub fn update_roots(&mut self) {
		let mut new_vertices: Vec<PoolEntry> = vec![];

		// first find the set of all destinations from the edges in the graph
  // a root is a vertex that is not a destination of any edge
		let destinations = self.edges
			.values()
			.filter_map(|edge| edge.destination)
			.collect::<HashSet<_>>();

		// now iterate over the current non-root vertices
  // and check if it is now a root based on the set of edge destinations
		for x in &self.vertices {
			if destinations.contains(&x.transaction_hash) {
				new_vertices.push(x.clone());
			} else {
				self.roots.push(x.clone());
			}
		}

		// now update our vertices to reflect the updated list
		self.vertices = new_vertices;
	}

	/// Adds a vertex and a set of incoming edges to the graph.
	///
	/// The PoolEntry at vertex is added to the graph; depending on the
	/// number of incoming edges, the vertex is either added to the vertices
	/// or to the roots.
	///
	/// Outgoing edges must not be included in edges; this method is designed
	/// for adding vertices one at a time and only accepts incoming edges as
	/// internal edges.
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

	/// add_vertex_only adds a vertex, meant to be complemented by add_edge_only
	/// in cases where delivering a vector of edges is not feasible or efficient
	pub fn add_vertex_only(&mut self, vertex: PoolEntry, is_root: bool) {
		if is_root {
			self.roots.push(vertex);
		} else {
			self.vertices.push(vertex);
		}
	}

	/// add_edge_only adds an edge
	pub fn add_edge_only(&mut self, edge: Edge) {
		self.edges.insert(edge.output_commitment(), edge);
	}

	/// Number of vertices (root + internal)
	pub fn len_vertices(&self) -> usize {
		self.vertices.len() + self.roots.len()
	}

	/// Number of root vertices only
	pub fn len_roots(&self) -> usize {
		self.roots.len()
	}

	/// Number of edges
	pub fn len_edges(&self) -> usize {
		self.edges.len()
	}

	/// Get the current list of roots
	pub fn get_roots(&self) -> Vec<core::hash::Hash> {
		self.roots.iter().map(|x| x.transaction_hash).collect()
	}

	/// Get list of all vertices in this graph including the roots
	pub fn get_vertices(&self) -> Vec<core::hash::Hash> {
		let mut hashes = self.roots
			.iter()
			.map(|x| x.transaction_hash)
			.collect::<Vec<_>>();
		let non_root_hashes = self.vertices
			.iter()
			.map(|x| x.transaction_hash)
			.collect::<Vec<_>>();
		hashes.extend(&non_root_hashes);
		return hashes;
	}
}

/// Using transaction merkle_inputs_outputs to calculate a deterministic hash;
/// this hashing mechanism has some ambiguity issues especially around range
/// proofs and any extra data the kernel may cover, but it is used initially
/// for testing purposes.
pub fn transaction_identifier(tx: &core::transaction::Transaction) -> core::hash::Hash {
	// core::transaction::merkle_inputs_outputs(&tx.inputs, &tx.outputs)
	tx.hash()
}

#[cfg(test)]
mod tests {
	use super::*;
	use util::secp;
	use keychain::Keychain;
	use rand;
	use core::core::SwitchCommitHash;

	#[test]
	fn test_add_entry() {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id1 = keychain.derive_key_id(1).unwrap();
		let key_id2 = keychain.derive_key_id(2).unwrap();
		let key_id3 = keychain.derive_key_id(3).unwrap();

		let output_commit = keychain.commit(70, &key_id1).unwrap();
		let switch_commit = keychain.switch_commit(&key_id1).unwrap();
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
		let inputs = vec![
			core::transaction::Input(keychain.commit(50, &key_id2).unwrap()),
			core::transaction::Input(keychain.commit(25, &key_id3).unwrap()),
		];
		let msg = secp::pedersen::ProofMessage::empty();
		let outputs = vec![
			core::transaction::Output {
				features: core::transaction::DEFAULT_OUTPUT,
				commit: output_commit,
				switch_commit_hash: switch_commit_hash,
				proof: keychain
					.range_proof(100, &key_id1, output_commit, msg)
					.unwrap(),
			},
		];
		let test_transaction = core::transaction::Transaction::new(inputs, outputs, 5, 0);

		let test_pool_entry = PoolEntry::new(&test_transaction);

		let incoming_edge_1 = Edge::new(
			Some(random_hash()),
			Some(core::hash::ZERO_HASH),
			output_commit,
		);

		let mut test_graph = DirectedGraph::empty();

		test_graph.add_entry(test_pool_entry, vec![incoming_edge_1]);

		assert_eq!(test_graph.vertices.len(), 1);
		assert_eq!(test_graph.roots.len(), 0);
		assert_eq!(test_graph.edges.len(), 1);
	}

	/// For testing/debugging: a random tx hash
	fn random_hash() -> core::hash::Hash {
		let hash_bytes: [u8; 32] = rand::random();
		core::hash::Hash(hash_bytes)
	}
}
