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

use pow::siphash::siphash24;
use pow::Proof;
use util;

trait EdgeType: PrimInt + ToPrimitive + Mul + BitOrAssign {}
impl EdgeType for u32 {}
impl EdgeType for u64 {}

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
	/// proof size
	proof_size: usize,
	/// define NIL type
	nil: T,
}

impl<T> Graph<T>
where
	T: EdgeType,
{
	/// Create a new graph with given parameters
	pub fn new(max_edges: T, max_sols: u32, proof_size: usize) -> Graph<T> {
		let max_nodes = 2 * max_edges.to_u64().unwrap();
		Graph {
			max_edges: max_edges,
			max_nodes: max_nodes,
			links: Vec::with_capacity(max_nodes as usize),
			adj_list: vec![T::from(T::max_value()).unwrap(); max_nodes as usize],
			visited: Bitmap::create(),
			max_sols: max_sols,
			solutions: vec![],
			proof_size: proof_size,
			nil: T::from(T::max_value()).unwrap(),
		}
	}

	/// Add an edge to the graph
	pub fn add_edge(&mut self, u: T, mut v: T) {
		v |= self.max_edges;
		assert!(u != self.nil && v != self.nil);
		let ulink = self.links.len();
		let vlink = self.links.len() + 1;
		self.links.push(Link {
			next: self.adj_list[u.to_u64().unwrap() as usize],
			to: u,
		});
		self.links.push(Link {
			next: self.adj_list[v.to_u64().unwrap() as usize],
			to: v,
		});
		self.adj_list[u.to_u64().unwrap() as usize] = T::from(ulink).unwrap();
		self.adj_list[v.to_u64().unwrap() as usize] = T::from(vlink).unwrap();
	}

	// remove lnk from u's adjacency list
	fn remove_adj(&mut self, u: T, lnk: T) {
		let mut lp = self.adj_list[u.to_u64().unwrap() as usize];
		while lp != lnk {
			assert!(lp != self.nil);
			lp = self.links[lp.to_u64().unwrap() as usize].next;
		}
		lp = self.links[lnk.to_u64().unwrap() as usize].next;
		self.links[lnk.to_u64().unwrap() as usize].to = self.nil;
	}

	fn remove_link(&mut self, lnk: T) {
		let u = self.links[lnk.to_u64().unwrap() as usize].to;
		if u == self.nil {
			return;
		}
		self.remove_adj(u, lnk);
		if self.adj_list[u.to_u64().unwrap() as usize] == self.nil {
			let mut rl = self.adj_list[(u ^ T::one()).to_u64().unwrap() as usize];
			while rl != self.nil {
				self.links[rl.to_u64().unwrap() as usize].to = self.nil;
				self.remove_link(rl ^ T::one());
				rl = self.links[rl.to_u64().unwrap() as usize].next;
			}
			self.adj_list[(u ^ T::one()).to_u64().unwrap() as usize] = self.nil;
		}
	}

	pub fn byte_count(&self) -> u64 {
		self.max_nodes * (mem::size_of::<Link<T>>() as u64 + mem::size_of::<T>() as u64)
			+ (self.max_edges.to_u64().unwrap() / 32) * mem::size_of::<u32>() as u64
	}

	fn test_bit(&mut self, u: u64) -> bool {
		self.visited.contains(u as u32)
	}

	fn cycles_with_link(&mut self, len: u32, u: T, dest: T) {
		if self.test_bit(u.to_u64().unwrap() >> 1) {
			println!("Already visited");
			return;
		}
		assert!(u != self.nil);
		if (u ^ T::one()) == dest {
			if len == self.proof_size as u32 {
				if self.solutions.len() < self.max_sols as usize {
					// create next solution
					self.solutions.push(Proof::zero(self.proof_size));
				}
				return;
			} else if len == self.proof_size as u32 {
				return;
			}
		}
		let mut au1 = self.adj_list[(u ^ T::one()).to_u64().unwrap() as usize];
		if au1 != self.nil {
			self.visited.add((u.to_u64().unwrap() >> 1) as u32);
			while au1 != self.nil {
				let i = self.solutions.len() - 1;
				// TODO: ???
				//self.solutions[i].nonces[len as usize] = au1.to_u64().unwrap() / 2;
				let link_index = (au1 ^ T::one()).to_u64().unwrap() as usize;
				let link = self.links[link_index].to;
				/*if link == self.nil {
					break;
				}*/
				self.cycles_with_link(len + 1, link, dest);
				au1 = self.links[au1.to_u64().unwrap() as usize].next;
			}
			self.visited.remove((u.to_u64().unwrap() >> 1) as u32);
		}
	}

	/// detect all cycles in the graph (up to proofsize)
	pub fn cycles(&mut self) -> usize {
		let mut n_links = self.links.len();
		self.solutions.push(Proof::zero(self.proof_size));
		while n_links > 0 {
			let sol_index = self.solutions.len() - 1;
			n_links -= 2;
			let u = self.links[n_links].to;
			let v = self.links[n_links + 1].to;
			if u != self.nil && v != self.nil {
				self.solutions[sol_index].nonces[0] = n_links as u64 / 2;
				self.cycles_with_link(1, u, v);
				self.remove_link(T::from(n_links).unwrap());
				self.remove_link(T::from(n_links + 1).unwrap());
			}
		}
		self.solutions.len() - 1
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
	proof_size: usize,
	edge_mask: T,
}

impl<T> CuckooContext<T>
where
	T: EdgeType,
{
	/// New Solver context
	pub fn new(
		edge_bits: u8,
		proof_size: usize,
		easiness_pct: u32,
		max_sols: u32,
	) -> CuckooContext<T> {
		let num_edges = 1 << edge_bits;
		let num_nodes = 2 * num_edges as u64;
		let easiness = easiness_pct.to_u64().unwrap() * num_nodes / 100;
		CuckooContext {
			siphash_keys: [0; 4],
			easiness: T::from(easiness).unwrap(),
			graph: Graph::new(T::from(num_edges).unwrap(), max_sols, proof_size),
			proof_size: proof_size,
			edge_mask: T::from(num_edges - 1).unwrap(),
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

	/// Return siphash masked for type
	pub fn sipnode(&self, edge: T, uorv: u64) -> T {
		let hash_u64 = siphash24(self.siphash_keys, 2 * edge.to_u64().unwrap() + uorv);
		let masked = hash_u64 & self.edge_mask.to_u64().unwrap();
		T::from(masked).unwrap()
	}

	/// Simple implementation of algorithm
	pub fn find_cycles_simple(&mut self, disp_callback: Option<fn(&Self)>) -> usize {
		for n in 0..self.easiness.to_u64().unwrap() {
			let u = self.sipnode(T::from(n).unwrap(), 0);
			let v = self.sipnode(T::from(n).unwrap(), 1);
			self.graph
				.add_edge(T::from(u).unwrap(), T::from(v).unwrap());
			if let Some(d) = disp_callback {
				d(&self);
			}
		}
		let n_sols = self.graph.cycles();
		n_sols
	}
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
	let num_sols = ctx_u32.find_cycles_simple(None);
	println!("Num sols found: {}", num_sols);
}
