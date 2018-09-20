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
use pow::num::{PrimInt, ToPrimitive};
use std::mem;
use std::ops::{BitOrAssign, Mul};

use blake2::blake2b::blake2b;
use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};
use croaring::Bitmap;
use std::io::Cursor;

use core::Proof;
use util;

trait EdgeType: PrimInt + ToPrimitive + Mul + BitOrAssign {}
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
	///
	visited: Bitmap,
	/// Maximum solutions
	max_sols: u32,
	///
	solutions: Vec<Proof>,
}

impl<T> Graph<T>
where
	T: EdgeType,
{
	/// Create a new graph with given parameters
	pub fn new(max_edges: T, max_sols: u32) -> Graph<T> {
		let max_nodes = 2 * max_edges.to_u64().unwrap();
		Graph {
			max_edges: max_edges,
			max_nodes: max_nodes,
			num_links: 0,
			links: Vec::with_capacity(max_nodes as usize),
			adj_list: Vec::with_capacity(max_nodes as usize),
			visited: Bitmap::create(),
			max_sols: max_sols,
			solutions: vec![],
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

	pub fn byte_count(&self) -> u64 {
		self.max_nodes * (mem::size_of::<Link<T>>() as u64 + mem::size_of::<T>() as u64)
			+ (self.max_edges.to_u64().unwrap() / 32) * mem::size_of::<u32>() as u64
	}
}

/// Cuckoo solver context
struct CuckooContext<T>
where
	T: EdgeType,
{
	siphash_keys: [u64; 4],
	easiness: T,
	graph: Graph<T>,
	proof_size: u8,
}

impl<T> CuckooContext<T>
where
	T: EdgeType,
{
	/// New Solver context
	pub fn new(
		edge_bits: u8,
		proof_size: u8,
		easiness_pct: u32,
		max_sols: u32,
	) -> CuckooContext<T> {
		let num_edges = 1 << edge_bits;
		let num_nodes = 2 * num_edges as u64;
		let easiness = easiness_pct.to_u64().unwrap() * num_nodes / 100;
		CuckooContext {
			siphash_keys: [0; 4],
			easiness: T::from(easiness).unwrap(),
			graph: Graph::new(T::from(num_edges).unwrap(), max_sols),
			proof_size: proof_size,
		}
	}

	/// Extract siphash keys from header
	pub fn create_keys(&mut self, header: Vec<u8>) {
		let h = blake2b(32, &[], &header);
		let hb = h.as_bytes();
		let mut rdr = Cursor::new(hb);
		self.siphash_keys = [
			rdr.read_u64::<BigEndian>().unwrap(),
			rdr.read_u64::<BigEndian>().unwrap(),
			rdr.read_u64::<BigEndian>().unwrap(),
			rdr.read_u64::<BigEndian>().unwrap(),
		];
	}

	/// Get a siphash key as a hex string (for display convenience)
	pub fn sipkey_hex(&self, index: usize) -> String {
		let mut rdr = vec![];
		rdr.write_u64::<LittleEndian>(self.siphash_keys[index])
			.unwrap();
		util::to_hex(rdr)
	}

	/// Return number of bytes used by the graph
	pub fn byte_count(&self) -> u64 {
		self.graph.byte_count()
	}

	/// Set the header and optional nonce in the last part of the header
	pub fn set_header_nonce(&mut self, mut header: Vec<u8>, nonce: Option<u32>) {
		let len = header.len();
		header.truncate(len - mem::size_of::<u32>());
		if let Some(n) = nonce {
			header.write_u32::<LittleEndian>(n).unwrap();
		}
		self.create_keys(header);
	}

	/// Simple implementation of algorithm
	pub fn find_cycles_simple(&mut self) {}
}

#[test]
fn cuckatoo() {
	let easiness_pct = 50;
	let nonce = 1546569;
	let range = 1;
	let header = [0u8; 80].to_vec();
	let proof_size = 42;
	let edge_bits = 15;
	let max_sols = 4;

	println!(
		"Looking for {}-cycle on cuckatoo{}(\"{}\",{}) with {}% edges",
		proof_size,
		edge_bits,
		String::from_utf8(header.clone()).unwrap(),
		nonce,
		easiness_pct
	);
	let mut ctx_u32 = CuckooContext::<u32>::new(edge_bits, proof_size, easiness_pct, max_sols);
	let mut bytes = ctx_u32.byte_count();
	let mut unit = 0;
	while bytes >= 10240 {
		bytes >>= 10;
		unit += 1;
	}
	println!("Using {}{}B memory", bytes, [' ', 'K', 'M', 'G', 'T'][unit]);
	ctx_u32.set_header_nonce(header, Some(nonce));
	println!(
		"Nonce {} k0 k1 k2 k3 {} {} {} {}",
		nonce,
		ctx_u32.sipkey_hex(0),
		ctx_u32.sipkey_hex(1),
		ctx_u32.sipkey_hex(2),
		ctx_u32.sipkey_hex(3)
	);
}
