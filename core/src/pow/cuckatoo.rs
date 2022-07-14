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
use crate::global;
use crate::pow::common::{CuckooParams, Link};
use crate::pow::error::Error;
use crate::pow::{PoWContext, Proof};
use byteorder::{BigEndian, WriteBytesExt};
use croaring::Bitmap;
use std::mem;
use util::ToHex;

struct Graph {
	/// Maximum number of edges
	max_edges: u64,
	/// Maximum nodes
	max_nodes: u64,
	/// Adjacency links
	links: Vec<Link>,
	/// Index into links array
	adj_list: Vec<u64>,
	///
	visited: Bitmap,
	/// Maximum solutions
	max_sols: u32,
	///
	pub solutions: Vec<Proof>,
	/// proof size
	proof_size: usize,
	/// define NIL type
	nil: u64,
}

impl Graph {
	/// Create a new graph with given parameters
	pub fn new(max_edges: u64, max_sols: u32, proof_size: usize) -> Result<Graph, Error> {
		if max_edges >= u64::max_value() / 2 {
			return Err(Error::Verification("graph is to big to build".to_string()));
		}
		let max_nodes = 2 * max_edges;
		Ok(Graph {
			max_edges,
			max_nodes,
			max_sols,
			proof_size,
			links: vec![],
			adj_list: vec![],
			visited: Bitmap::create(),
			solutions: vec![],
			nil: u64::max_value(),
		})
	}

	pub fn reset(&mut self) -> Result<(), Error> {
		//TODO: Can be optimised
		self.links = Vec::with_capacity(2 * self.max_nodes as usize);
		self.adj_list = vec![u64::max_value(); 2 * self.max_nodes as usize];
		self.solutions = vec![Proof::zero(self.proof_size); 1];
		self.visited = Bitmap::create();
		Ok(())
	}

	pub fn byte_count(&self) -> Result<u64, Error> {
		Ok(2 * self.max_edges * mem::size_of::<Link>() as u64
			+ mem::size_of::<u64>() as u64 * 2 * self.max_nodes)
	}

	/// Add an edge to the graph
	pub fn add_edge(&mut self, u: u64, mut v: u64) -> Result<(), Error> {
		if u >= self.max_nodes || v >= self.max_nodes {
			return Err(Error::EdgeAddition);
		}
		v = v + self.max_nodes;
		let adj_u = self.adj_list[(u ^ 1) as usize];
		let adj_v = self.adj_list[(v ^ 1) as usize];
		if adj_u != self.nil && adj_v != self.nil {
			let sol_index = self.solutions.len() - 1;
			self.solutions[sol_index].nonces[0] = self.links.len() as u64 / 2;
			self.cycles_with_link(1, u, v)?;
		}
		let ulink = self.links.len() as u64;
		let vlink = (self.links.len() + 1) as u64;
		if vlink == self.nil {
			return Err(Error::EdgeAddition);
		}
		self.links.push(Link {
			next: self.adj_list[u as usize],
			to: u,
		});
		self.links.push(Link {
			next: self.adj_list[v as usize],
			to: v,
		});
		self.adj_list[u as usize] = ulink;
		self.adj_list[v as usize] = vlink;
		Ok(())
	}

	fn test_bit(&mut self, u: u64) -> bool {
		self.visited.contains(u as u32)
	}

	fn cycles_with_link(&mut self, len: u32, u: u64, dest: u64) -> Result<(), Error> {
		if self.test_bit(u >> 1) {
			return Ok(());
		}
		if (u ^ 1) == dest {
			if len == self.proof_size as u32 {
				if self.solutions.len() < self.max_sols as usize {
					// create next solution
					self.solutions.push(Proof::zero(self.proof_size));
				}
				return Ok(());
			}
		} else if len == self.proof_size as u32 {
			return Ok(());
		}
		let mut au1 = self.adj_list[(u ^ 1) as usize];
		if au1 != self.nil {
			self.visited.add((u >> 1) as u32);
			while au1 != self.nil {
				let i = self.solutions.len() - 1;
				self.solutions[i].nonces[len as usize] = au1 / 2;
				let link_index = (au1 ^ 1) as usize;
				let link = self.links[link_index].to;
				if link != self.nil {
					self.cycles_with_link(len + 1, link, dest)?;
				}
				au1 = self.links[au1 as usize].next;
			}
			self.visited.remove((u >> 1) as u32);
		}
		Ok(())
	}
}

/// Instantiate a new CuckatooContext as a PowContext. Note that this can't
/// be moved in the PoWContext trait as this particular trait needs to be
/// convertible to an object trait.
pub fn new_cuckatoo_ctx(
	edge_bits: u8,
	proof_size: usize,
	max_sols: u32,
) -> Result<Box<dyn PoWContext>, Error> {
	Ok(Box::new(CuckatooContext::new_impl(
		edge_bits, proof_size, max_sols,
	)?))
}

/// Cuckatoo solver context
pub struct CuckatooContext {
	params: CuckooParams,
	graph: Graph,
}

impl PoWContext for CuckatooContext {
	fn set_header_nonce(
		&mut self,
		header: Vec<u8>,
		nonce: Option<u32>,
		solve: bool,
	) -> Result<(), Error> {
		self.set_header_nonce_impl(header, nonce, solve)
	}

	fn find_cycles(&mut self) -> Result<Vec<Proof>, Error> {
		let num_edges = self.params.num_edges;
		self.find_cycles_iter(0..num_edges)
	}

	fn verify(&self, proof: &Proof) -> Result<(), Error> {
		self.verify_impl(proof)
	}
}

impl CuckatooContext {
	/// New Solver context
	pub fn new_impl(
		edge_bits: u8,
		proof_size: usize,
		max_sols: u32,
	) -> Result<CuckatooContext, Error> {
		let params = CuckooParams::new(edge_bits, edge_bits, proof_size)?;
		let num_edges = params.num_edges;
		Ok(CuckatooContext {
			params,
			graph: Graph::new(num_edges, max_sols, proof_size)?,
		})
	}

	/// Get a siphash key as a hex string (for display convenience)
	pub fn sipkey_hex(&self, index: usize) -> Result<String, Error> {
		let mut rdr = vec![];
		rdr.write_u64::<BigEndian>(self.params.siphash_keys[index])?;
		Ok(rdr.to_hex())
	}

	/// Return number of bytes used by the graph
	pub fn byte_count(&self) -> Result<u64, Error> {
		self.graph.byte_count()
	}

	/// Set the header and optional nonce in the last part of the header
	pub fn set_header_nonce_impl(
		&mut self,
		header: Vec<u8>,
		nonce: Option<u32>,
		solve: bool,
	) -> Result<(), Error> {
		self.params.reset_header_nonce(header, nonce)?;
		if solve {
			self.graph.reset()?;
		}
		Ok(())
	}

	/// Simple implementation of algorithm
	pub fn find_cycles_iter<I>(&mut self, iter: I) -> Result<Vec<Proof>, Error>
	where
		I: Iterator<Item = u64>,
	{
		let mut val = vec![];
		for n in iter {
			val.push(n);
			let u = self.params.sipnode(n, 0)?;
			let v = self.params.sipnode(n, 1)?;
			self.graph.add_edge(u, v)?;
		}
		self.graph.solutions.pop();
		for s in &mut self.graph.solutions {
			s.nonces = map_vec!(s.nonces, |n| val[*n as usize]);
			s.nonces.sort_unstable();
		}
		for s in &self.graph.solutions {
			self.verify_impl(&s)?;
		}
		if self.graph.solutions.is_empty() {
			Err(Error::NoSolution)
		} else {
			Ok(self.graph.solutions.clone())
		}
	}

	/// Verify that given edges are ascending and form a cycle in a header-generated
	/// graph
	pub fn verify_impl(&self, proof: &Proof) -> Result<(), Error> {
		let size = proof.proof_size();
		if size != global::proofsize() {
			return Err(Error::Verification("wrong cycle length".to_owned()));
		}
		let nonces = &proof.nonces;
		let mut uvs = vec![0u64; 2 * size];
		let mask = u64::MAX >> size.leading_zeros(); // round size up to 2-power - 1
		let mut xor0: u64 = (size as u64 / 2) & 1;
		let mut xor1: u64 = xor0;
		// the next two arrays form a linked list of nodes with matching bits 6..1
		let mut headu = vec![2 * size; 1 + mask as usize];
		let mut headv = vec![2 * size; 1 + mask as usize];
		let mut prev = vec![0usize; 2 * size];

		for n in 0..size {
			if nonces[n] > self.params.edge_mask {
				return Err(Error::Verification("edge too big".to_owned()));
			}
			if n > 0 && nonces[n] <= nonces[n - 1] {
				return Err(Error::Verification("edges not ascending".to_owned()));
			}
			let u = self.params.sipnode(nonces[n], 0)?;
			let v = self.params.sipnode(nonces[n], 1)?;

			uvs[2 * n] = u;
			let ubits = (u >> 1 & mask) as usize; // larger shifts work too, up to edgebits-6
			prev[2 * n] = headu[ubits];
			headu[ubits] = 2 * n;

			uvs[2 * n + 1] = v;
			let vbits = (v >> 1 & mask) as usize;
			prev[2 * n + 1] = headv[vbits];
			headv[vbits] = 2 * n + 1;

			xor0 ^= u;
			xor1 ^= v;
		}
		if xor0 | xor1 != 0 {
			return Err(Error::Verification("endpoints don't match up".to_owned()));
		}
		// make prev lists circular
		for n in 0..size {
			if prev[2 * n] == 2 * size {
				let ubits = (uvs[2 * n] >> 1 & mask) as usize;
				prev[2 * n] = headu[ubits];
			}
			if prev[2 * n + 1] == 2 * size {
				let vbits = (uvs[2 * n + 1] >> 1 & mask) as usize;
				prev[2 * n + 1] = headv[vbits];
			}
		}
		let mut n = 0;
		let mut i = 0;
		let mut j;
		loop {
			// follow cycle
			j = i;
			let mut k = j;
			loop {
				k = prev[k];
				if k == i {
					break;
				}
				if uvs[k] >> 1 == uvs[i] >> 1 {
					// find other edge endpoint matching one at i
					if j != i {
						return Err(Error::Verification("branch in cycle".to_owned()));
					}
					j = k;
				}
			}
			if j == i || uvs[j] == uvs[i] {
				return Err(Error::Verification("cycle dead ends".to_owned()));
			}
			i = j ^ 1;
			n += 1;
			if i == 0 {
				break;
			}
		}
		if n == size {
			Ok(())
		} else {
			Err(Error::Verification("cycle too short".to_owned()))
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	// Cuckatoo 29 Solution for Header [0u8;80] - nonce 20
	static V1_29: [u64; 42] = [
		0x48a9e2, 0x9cf043, 0x155ca30, 0x18f4783, 0x248f86c, 0x2629a64, 0x5bad752, 0x72e3569,
		0x93db760, 0x97d3b37, 0x9e05670, 0xa315d5a, 0xa3571a1, 0xa48db46, 0xa7796b6, 0xac43611,
		0xb64912f, 0xbb6c71e, 0xbcc8be1, 0xc38a43a, 0xd4faa99, 0xe018a66, 0xe37e49c, 0xfa975fa,
		0x11786035, 0x1243b60a, 0x12892da0, 0x141b5453, 0x1483c3a0, 0x1505525e, 0x1607352c,
		0x16181fe3, 0x17e3a1da, 0x180b651e, 0x1899d678, 0x1931b0bb, 0x19606448, 0x1b041655,
		0x1b2c20ad, 0x1bd7a83c, 0x1c05d5b0, 0x1c0b9caa,
	];

	// Cuckatoo 31 Solution for Header [0u8;80] - nonce 99
	static V1_31: [u64; 42] = [
		0x1128e07, 0xc181131, 0x110fad36, 0x1135ddee, 0x1669c7d3, 0x1931e6ea, 0x1c0005f3,
		0x1dd6ecca, 0x1e29ce7e, 0x209736fc, 0x2692bf1a, 0x27b85aa9, 0x29bb7693, 0x2dc2a047,
		0x2e28650a, 0x2f381195, 0x350eb3f9, 0x3beed728, 0x3e861cbc, 0x41448cc1, 0x41f08f6d,
		0x42fbc48a, 0x4383ab31, 0x4389c61f, 0x4540a5ce, 0x49a17405, 0x50372ded, 0x512f0db0,
		0x588b6288, 0x5a36aa46, 0x5c29e1fe, 0x6118ab16, 0x634705b5, 0x6633d190, 0x6683782f,
		0x6728b6e1, 0x67adfb45, 0x68ae2306, 0x6d60f5e1, 0x78af3c4f, 0x7dde51ab, 0x7faced21,
	];

	// Cuckatoo 32 Solution for Header [0u8;80] - nonce 17
	static V1_32: [u64; 42] = [
		0x6da0bbf, 0xb175276, 0xf978803, 0x187bea71, 0x2074a1a6, 0x22270923, 0x2c70b560,
		0x411d193f, 0x417c55d4, 0x4ebbda62, 0x5238584a, 0x545efac9, 0x569e98e1, 0x57040b66,
		0x5e16153e, 0x5e749d2e, 0x60b771c2, 0x68e63420, 0x74a2825e, 0x755790ac, 0x7d5e280f,
		0x7fe4d148, 0x934b32c8, 0x94a0c441, 0x9643fb25, 0x9718e41d, 0x982e6b8b, 0x9c47d21c,
		0xa1f64135, 0xa90e209c, 0xabb868cb, 0xafef989e, 0xb0fc021e, 0xb20a7b56, 0xb5e59931,
		0xb63e46b9, 0xb8823ed5, 0xd11e966c, 0xd95e515d, 0xe0245efe, 0xf3edc79a, 0xfb8a29ce,
	];

	// Cuckatoo 33 Solution for Header [0u8;80] - nonce 79
	static V1_33: [u64; 42] = [
		0x7aaf51f,
		0x1434ebf3,
		0x25bcee6e,
		0x2fbddf0b,
		0x322a87b6,
		0x414f6a57,
		0x701a84af,
		0x7c432040,
		0x822b8ee0,
		0x83c9fed3,
		0x89af26b2,
		0xa5bc5d69,
		0xbe924630,
		0xd3146f50,
		0xd4e0f240,
		0xe10e5bdc,
		0x113400ccc,
		0x114a917b2,
		0x118482498,
		0x11deca0f4,
		0x1241c7ff0,
		0x1245f8886,
		0x12a6517e3,
		0x12c1a0edd,
		0x142d988ee,
		0x14637a89b,
		0x15399e735,
		0x1699c1cf9,
		0x16e91ddd4,
		0x17414f603,
		0x18c07384c,
		0x1993cdd97,
		0x19d37ce5b,
		0x1a43455c5,
		0x1aa312c2f,
		0x1b20fe128,
		0x1b7610376,
		0x1bce4d125,
		0x1c4834307,
		0x1c7a2e5b2,
		0x1da840832,
		0x1e4e3da0c,
	];

	#[test]
	fn cuckatoo() {
		global::set_local_chain_type(global::ChainTypes::Mainnet);
		let ret = basic_solve();
		if let Err(r) = ret {
			panic!("basic_solve: Error: {}", r);
		}
		let ret = validate29_vectors();
		if let Err(r) = ret {
			panic!("validate_29_vectors: Error: {}", r);
		}
		let ret = validate31_vectors();
		if let Err(r) = ret {
			panic!("validate_31_vectors: Error: {}", r);
		}
		let ret = validate32_vectors();
		if let Err(r) = ret {
			panic!("validate_32_vectors: Error: {}", r);
		}
		let ret = validate33_vectors();
		if let Err(r) = ret {
			panic!("validate_33_vectors: Error: {}", r);
		}
		let ret = validate_fail();
		if let Err(r) = ret {
			panic!("validate_fail: Error: {}", r);
		}
	}

	fn validate29_vectors() -> Result<(), Error> {
		let mut ctx = CuckatooContext::new_impl(29, 42, 10).unwrap();
		ctx.set_header_nonce([0u8; 80].to_vec(), Some(20), false)?;
		assert!(ctx.verify(&Proof::new(V1_29.to_vec())).is_ok());
		Ok(())
	}

	fn validate31_vectors() -> Result<(), Error> {
		let mut ctx = CuckatooContext::new_impl(31, 42, 10).unwrap();
		ctx.set_header_nonce([0u8; 80].to_vec(), Some(99), false)?;
		assert!(ctx.verify(&Proof::new(V1_31.to_vec())).is_ok());
		Ok(())
	}

	fn validate32_vectors() -> Result<(), Error> {
		let mut ctx = CuckatooContext::new_impl(32, 42, 10).unwrap();
		ctx.set_header_nonce([0u8; 80].to_vec(), Some(17), false)?;
		assert!(ctx.verify(&Proof::new(V1_32.to_vec())).is_ok());
		Ok(())
	}

	fn validate33_vectors() -> Result<(), Error> {
		let mut ctx = CuckatooContext::new_impl(33, 42, 10).unwrap();
		ctx.set_header_nonce([0u8; 80].to_vec(), Some(79), false)?;
		assert!(ctx.verify(&Proof::new(V1_33.to_vec())).is_ok());
		Ok(())
	}

	fn validate_fail() -> Result<(), Error> {
		let mut ctx = CuckatooContext::new_impl(29, 42, 10).unwrap();
		let mut header = [0u8; 80];
		header[0] = 1u8;
		ctx.set_header_nonce(header.to_vec(), Some(20), false)?;
		assert!(ctx.verify(&Proof::new(V1_29.to_vec())).is_err());
		header[0] = 0u8;
		ctx.set_header_nonce(header.to_vec(), Some(20), false)?;
		assert!(ctx.verify(&Proof::new(V1_29.to_vec())).is_ok());
		let mut bad_proof = V1_29;
		bad_proof[0] = 0x48a9e1;
		assert!(ctx.verify(&Proof::new(bad_proof.to_vec())).is_err());
		Ok(())
	}

	fn basic_solve() -> Result<(), Error> {
		let nonce = 1546569;
		let _range = 1;
		let header = [0u8; 80].to_vec();
		let proof_size = 42;
		let edge_bits = 15;
		let max_sols = 4;

		println!(
			"Looking for {}-cycle on cuckatoo{}(\"{}\",{})",
			proof_size,
			edge_bits,
			String::from_utf8(header.clone()).unwrap(),
			nonce
		);
		let mut ctx_u32 = CuckatooContext::new_impl(edge_bits, proof_size, max_sols)?;
		let mut bytes = ctx_u32.byte_count()?;
		let mut unit = 0;
		while bytes >= 10240 {
			bytes >>= 10;
			unit += 1;
		}
		println!("Using {}{}B memory", bytes, [' ', 'K', 'M', 'G', 'T'][unit]);
		ctx_u32.set_header_nonce(header, Some(nonce), true)?;
		println!(
			"Nonce {} k0 k1 k2 k3 {} {} {} {}",
			nonce,
			ctx_u32.sipkey_hex(0)?,
			ctx_u32.sipkey_hex(1)?,
			ctx_u32.sipkey_hex(2)?,
			ctx_u32.sipkey_hex(3)?
		);
		let sols = ctx_u32.find_cycles()?;
		// We know this nonce has 2 solutions
		assert_eq!(sols.len(), 2);
		for s in sols {
			println!("{:?}", s);
		}
		Ok(())
	}
}
