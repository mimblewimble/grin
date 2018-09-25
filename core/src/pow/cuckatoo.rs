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
use std::ops::{BitOrAssign, Mul};
use std::{fmt, mem};

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
#[derive(Debug, Clone, Eq, PartialEq)]
struct Link<T>
where
	T: EdgeType,
{
	next: T,
	to: T,
}

impl<T> fmt::Display for Link<T>
where
	T: EdgeType,
{
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(
			f,
			"(next: {}, to: {})",
			self.next.to_u64().unwrap(),
			self.to.to_u64().unwrap()
		)
	}
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
	pub solutions: Vec<Proof>,
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
			links: Vec::with_capacity(2 * max_nodes as usize),
			adj_list: vec![T::from(T::max_value()).unwrap(); 2 * max_nodes as usize],
			visited: Bitmap::create(),
			max_sols: max_sols,
			solutions: vec![Proof::zero(proof_size); 1],
			proof_size: proof_size,
			nil: T::from(T::max_value()).unwrap(),
		}
	}

	/// Add an edge to the graph
	pub fn add_edge(&mut self, u: T, mut v: T) {
		assert!(u < T::from(self.max_nodes).unwrap());
		assert!(v < T::from(self.max_nodes).unwrap());
		v = v + T::from(self.max_nodes).unwrap();
		let adj_u = self.adj_list[(u ^ T::one()).to_u64().unwrap() as usize];
		let adj_v = self.adj_list[(v ^ T::one()).to_u64().unwrap() as usize];
		if adj_u != self.nil && adj_v != self.nil {
			let sol_index = self.solutions.len() - 1;
			self.solutions[sol_index].nonces[0] = self.links.len() as u64 / 2;
			self.cycles_with_link(1, u, v);
		}
		let ulink = self.links.len();
		let vlink = self.links.len() + 1;
		assert!(T::from(vlink).unwrap() != self.nil);
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

	pub fn byte_count(&self) -> u64 {
		2 * self.max_edges.to_u64().unwrap() * mem::size_of::<Link<T>>() as u64
			+ mem::size_of::<T>() as u64 * 2 * self.max_nodes
	}

	fn test_bit(&mut self, u: u64) -> bool {
		self.visited.contains(u as u32)
	}

	fn cycles_with_link(&mut self, len: u32, u: T, dest: T) {
		if self.test_bit((u >> 1).to_u64().unwrap()) {
			return;
		}
		if (u ^ T::one()) == dest {
			println!("{}-cycle found", len);
			if len == self.proof_size as u32 {
				if self.solutions.len() < self.max_sols as usize {
					// create next solution
					self.solutions.push(Proof::zero(self.proof_size));
				}
				return;
			}
		} else if len == self.proof_size as u32 {
			return;
		}
		let mut au1 = self.adj_list[(u ^ T::one()).to_u64().unwrap() as usize];
		if au1 != self.nil {
			self.visited.add((u >> 1).to_u64().unwrap() as u32);
			while au1 != self.nil {
				let i = self.solutions.len() - 1;
				self.solutions[i].nonces[len as usize] = au1.to_u64().unwrap() / 2;
				let link_index = (au1 ^ T::one()).to_u64().unwrap() as usize;
				let link = self.links[link_index].to;
				if link != self.nil {
					self.cycles_with_link(len + 1, link, dest);
				}
				au1 = self.links[au1.to_u64().unwrap() as usize].next;
			}
			self.visited.remove((u >> 1).to_u64().unwrap() as u32);
		}
	}
}

/// Cuckoo solver context
struct CuckooContext<T>
where
	T: EdgeType,
{
	siphash_keys: [u64; 4],
	easiness: T,
	pub graph: Graph<T>,
	proof_size: usize,
	edge_mask: T,
	num_edges: T,
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
			num_edges: T::from(num_edges).unwrap(),
		}
	}

	/// Extract siphash keys from header
	pub fn create_keys(&mut self, header: Vec<u8>) {
		let h = blake2b(32, &[], &header);
		let hb = h.as_bytes();
		let mut rdr = Cursor::new(hb);
		self.siphash_keys = [
			rdr.read_u64::<LittleEndian>().unwrap(),
			rdr.read_u64::<LittleEndian>().unwrap(),
			rdr.read_u64::<LittleEndian>().unwrap(),
			rdr.read_u64::<LittleEndian>().unwrap(),
		];
	}

	/// Get a siphash key as a hex string (for display convenience)
	pub fn sipkey_hex(&self, index: usize) -> String {
		let mut rdr = vec![];
		rdr.write_u64::<BigEndian>(self.siphash_keys[index])
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
		let n_sols = self.graph.solutions.len() - 1;
		for s in &mut self.graph.solutions {
			s.nonces.sort();
		}
		for s in &self.graph.solutions {
			println!("Verification: {}", self.verify(&s));
		}
		n_sols
	}

	/// Verify that given edges are ascending and form a cycle in a header-generated
	/// graph
	pub fn verify(&self, proof: &Proof) -> bool {
		let nonces = &proof.nonces;
		let mut uvs = vec![0u64; 2 * proof.proof_size()];
		let mut xor0: u64 = (self.proof_size as u64 / 2) & 1;
		let mut xor1: u64 = xor0;

		for n in 0..proof.proof_size() {
			if nonces[n] > self.edge_mask.to_u64().unwrap() {
				// POW TOO BIG
				println!("TOO BIG");
				return false;
			}
			if n > 0 && nonces[n] <= nonces[n - 1] {
				// POW TOO SMALL
				println!("TOO SMALL");
				return false;
			}
			uvs[2 * n] = self
				.sipnode(T::from(nonces[n]).unwrap(), 0)
				.to_u64()
				.unwrap();
			uvs[2 * n + 1] = self
				.sipnode(T::from(nonces[n]).unwrap(), 1)
				.to_u64()
				.unwrap();
			xor0 ^= uvs[2 * n];
			xor1 ^= uvs[2 * n + 1];
		}
		if xor0 | xor1 != 0 {
			// POW NON MATCHING
			println!("NON MATCHING");
			return false;
		}
		let mut n = 0;
		let mut i = 0;
		let mut j;
		loop {
			// follow cycle
			j = i;
			let mut k = j;
			loop {
				k = (k + 2) % (2 * self.proof_size);
				if k == i {
					break;
				}
				if uvs[k] >> 1 == uvs[i] >> 1 {
					// find other edge endpoint matching one at i
					if j != i {
						// POW_BRANCH
						println!("POW_BRANCH"); // already found one before
						return false;
					}
					j = k;
				}
			}
			if j == i || uvs[j] == uvs[i] {
				// POW_DEAD_END
				println!("POW_DEAD_END");
				return false;
			}
			i = j ^ 1;
			n += 1;
			if i == 0 {
				break;
			}
		}
		if n == self.proof_size {
			true
		} else {
			//POW_SHORT_CYCLE
			println!("POW_SHORT_CYCLE");
			false
		}
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
	for i in 0..num_sols {
		let sol = ctx_u32.graph.solutions[i].clone();
		println!("{:?}", sol);
	}
}
