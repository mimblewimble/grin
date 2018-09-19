// Copyright 2018 The Grin Developers
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

//! Implementation of Cuckatoo Cycle designed by John Tromp.
use std::ops::{Mul, BitOrAssign};
use pow::num::{ToPrimitive, PrimInt};

trait EdgeType: PrimInt + ToPrimitive + Mul + BitOrAssign{}
impl EdgeType for u32 {}
impl EdgeType for u64 {}

/// An edge in the Cuckatoo/Cuckoo graph, reference to 2 endpoints
struct Edge<T>
where
	T: EdgeType,
{
	u: T,
	v: T,
}

/// An element of an adjencency list
struct Link<T>
where
	T: EdgeType,
{
	next: T,
	to: T,
}

struct Graph<T>
where
	T: EdgeType,
{
	/// Maximum number of edges
	max_edges: T,
	/// Maximum nodes
	max_nodes: u64,
	/// AKA halfedges, twice the number of edges
	num_links: u64,
	/// Adjacency links
	links: Vec<Link<T>>,
	/// Index into links array
	adj_list: Vec<T>,
	// TODO:
	// bitmap<u32> visited;
	// TODO:
	/// Maximum solutions
	max_sols: u32,
	// TODO:
	// proof* sols
	/// Number of solutions in the graph
	num_sols: u32,
}

impl<T> Graph<T>
where
	T: EdgeType,
{
	/// Create a new graph with given parameters
	pub fn new(max_edges: T, max_sols: u32) -> Graph<T> {
		Graph {
			max_edges: max_edges,
			max_nodes: 2 * max_edges.to_u64().unwrap(),
			num_links: 0,
			links: vec![],
			adj_list: vec![],
			max_sols: max_sols,
			num_sols: 0,
		}
	}

	/// Add an edge to the graph
	pub fn add_edge(&mut self, u: T, mut v: T) {
		v |= self.max_edges;
		self.num_links += 1;
		let ulink = self.num_links;
		// the two halfedges of an edge differ only in last bit
		self.num_links += 1;
		let vlink = self.num_links;
		self.links[ulink as usize].next = self.adj_list[u.to_u64().unwrap() as usize];
		self.links[vlink as usize].next = self.adj_list[v.to_u64().unwrap() as usize];
		//TODO: Incomplete
		//self.adj_list[u.to_u64().unwrap() as usize] = ulink;
		//self.adj_list[v.to_u64().unwrap() as usize] = vlink;

	}
}

/// Cuckoo solver context
struct CuckooContext<T>
where
	T: EdgeType
{
	siphash_keys: [u64; 4],
	easiness: T,
	graph: Graph<T>,
}

impl<T> CuckooContext<T>
where
	T: EdgeType,
{
	/// New Solver context
	pub fn new(header:&[u8], nonce: u32, easiness:T, max_edges: T, max_sols: u32) -> CuckooContext<T> {
		CuckooContext {
			siphash_keys: [0; 4],
			easiness: easiness,
			graph: Graph::new(max_edges, max_sols),
		}
	}
}

#[test]
fn cuckatoo() {
	let ctx_u32 = CuckooContext::new(&[0u8;3], 0, 100u32, 10u32, 10);
	let ctx_u64 = CuckooContext::new(&[0u8;3], 0, 100u64, 10u64, 10);
}
