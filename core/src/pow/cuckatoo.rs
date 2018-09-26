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
use pow::num::ToPrimitive;
use std::mem;

use byteorder::{BigEndian, LittleEndian, WriteBytesExt};
use croaring::Bitmap;

use pow::common::EdgeType;
use pow::common::{self, Link};
use pow::error::{Error, ErrorKind};
use pow::{PoWContext, Proof};
use util;

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
	pub fn new(max_edges: T, max_sols: u32, proof_size: usize) -> Result<Graph<T>, Error> {
		let max_nodes = 2 * max_edges.to_u64().ok_or(ErrorKind::IntegerCast)?;
		Ok(Graph {
			max_edges: max_edges,
			max_nodes: max_nodes,
			links: vec![],
			adj_list: vec![],
			visited: Bitmap::create(),
			max_sols: max_sols,
			solutions: vec![],
			proof_size: proof_size,
			nil: T::from(T::max_value()).ok_or(ErrorKind::IntegerCast)?,
		})
	}

	pub fn reset(&mut self) -> Result<(), Error> {
		//TODO: Can be optimised
		self.links = Vec::with_capacity(2 * self.max_nodes as usize);
		self.adj_list = vec![
			T::from(T::max_value()).ok_or(ErrorKind::IntegerCast)?;
			2 * self.max_nodes as usize
		];
		self.solutions = vec![Proof::zero(self.proof_size); 1];
		self.visited = Bitmap::create();
		Ok(())
	}

	pub fn byte_count(&self) -> Result<u64, Error> {
		Ok(2
			* self.max_edges.to_u64().ok_or(ErrorKind::IntegerCast)?
			* mem::size_of::<Link<T>>() as u64 + mem::size_of::<T>() as u64 * 2 * self.max_nodes)
	}

	/// Add an edge to the graph
	pub fn add_edge(&mut self, u: T, mut v: T) -> Result<(), Error> {
		let max_nodes_t = T::from(self.max_nodes).ok_or(ErrorKind::IntegerCast)?;
		if u >= max_nodes_t || v >= max_nodes_t {
			return Err(ErrorKind::EdgeAddition)?;
		}
		v = v + T::from(self.max_nodes).ok_or(ErrorKind::IntegerCast)?;
		let adj_u = self.adj_list[(u ^ T::one()).to_u64().ok_or(ErrorKind::IntegerCast)? as usize];
		let adj_v = self.adj_list[(v ^ T::one()).to_u64().ok_or(ErrorKind::IntegerCast)? as usize];
		if adj_u != self.nil && adj_v != self.nil {
			let sol_index = self.solutions.len() - 1;
			self.solutions[sol_index].nonces[0] = self.links.len() as u64 / 2;
			self.cycles_with_link(1, u, v)?;
		}
		let ulink = self.links.len();
		let vlink = self.links.len() + 1;
		if T::from(vlink).ok_or(ErrorKind::IntegerCast)? == self.nil {
			return Err(ErrorKind::EdgeAddition)?;
		}
		self.links.push(Link {
			next: self.adj_list[u.to_u64().ok_or(ErrorKind::IntegerCast)? as usize],
			to: u,
		});
		self.links.push(Link {
			next: self.adj_list[v.to_u64().ok_or(ErrorKind::IntegerCast)? as usize],
			to: v,
		});
		self.adj_list[u.to_u64().ok_or(ErrorKind::IntegerCast)? as usize] =
			T::from(ulink).ok_or(ErrorKind::IntegerCast)?;
		self.adj_list[v.to_u64().ok_or(ErrorKind::IntegerCast)? as usize] =
			T::from(vlink).ok_or(ErrorKind::IntegerCast)?;
		Ok(())
	}

	fn test_bit(&mut self, u: u64) -> bool {
		self.visited.contains(u as u32)
	}

	fn cycles_with_link(&mut self, len: u32, u: T, dest: T) -> Result<(), Error> {
		if self.test_bit((u >> 1).to_u64().ok_or(ErrorKind::IntegerCast)?) {
			return Ok(());
		}
		if (u ^ T::one()) == dest {
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
		let mut au1 =
			self.adj_list[(u ^ T::one()).to_u64().ok_or(ErrorKind::IntegerCast)? as usize];
		if au1 != self.nil {
			self.visited
				.add((u >> 1).to_u64().ok_or(ErrorKind::IntegerCast)? as u32);
			while au1 != self.nil {
				let i = self.solutions.len() - 1;
				self.solutions[i].nonces[len as usize] =
					au1.to_u64().ok_or(ErrorKind::IntegerCast)? / 2;
				let link_index = (au1 ^ T::one()).to_u64().ok_or(ErrorKind::IntegerCast)? as usize;
				let link = self.links[link_index].to;
				if link != self.nil {
					self.cycles_with_link(len + 1, link, dest)?;
				}
				au1 = self.links[au1.to_u64().ok_or(ErrorKind::IntegerCast)? as usize].next;
			}
			self.visited
				.remove((u >> 1).to_u64().ok_or(ErrorKind::IntegerCast)? as u32);
		}
		Ok(())
	}
}

/// Cuckatoo solver context
pub struct CuckatooContext<T>
where
	T: EdgeType,
{
	siphash_keys: [u64; 4],
	easiness: T,
	graph: Graph<T>,
	proof_size: usize,
	edge_mask: T,
}

impl<T> PoWContext<T> for CuckatooContext<T>
where
	T: EdgeType,
{
	fn new(
		edge_bits: u8,
		proof_size: usize,
		easiness_pct: u32,
		max_sols: u32,
	) -> Result<Box<Self>, Error> {
		Ok(Box::new(CuckatooContext::<T>::new_impl(
			edge_bits,
			proof_size,
			easiness_pct,
			max_sols,
		)?))
	}

	fn set_header_nonce(&mut self, header: Vec<u8>, nonce: Option<u32>) -> Result<(), Error> {
		self.set_header_nonce_impl(header, nonce)
	}

	fn find_cycles(&mut self) -> Result<Vec<Proof>, Error> {
		self.find_cycles_impl()
	}

	fn verify(&self, proof: &Proof) -> Result<(), Error> {
		self.verify_impl(proof)
	}
}

impl<T> CuckatooContext<T>
where
	T: EdgeType,
{
	/// New Solver context
	pub fn new_impl(
		edge_bits: u8,
		proof_size: usize,
		easiness_pct: u32,
		max_sols: u32,
	) -> Result<CuckatooContext<T>, Error> {
		let num_edges = 1 << edge_bits;
		let num_nodes = 2 * num_edges as u64;
		let easiness = easiness_pct.to_u64().ok_or(ErrorKind::IntegerCast)? * num_nodes / 100;
		Ok(CuckatooContext {
			siphash_keys: [0; 4],
			easiness: T::from(easiness).ok_or(ErrorKind::IntegerCast)?,
			graph: Graph::new(
				T::from(num_edges).ok_or(ErrorKind::IntegerCast)?,
				max_sols,
				proof_size,
			)?,
			proof_size: proof_size,
			edge_mask: T::from(num_edges - 1).ok_or(ErrorKind::IntegerCast)?,
		})
	}

	/// Get a siphash key as a hex string (for display convenience)
	pub fn sipkey_hex(&self, index: usize) -> Result<String, Error> {
		let mut rdr = vec![];
		rdr.write_u64::<BigEndian>(self.siphash_keys[index])?;
		Ok(util::to_hex(rdr))
	}

	/// Return number of bytes used by the graph
	pub fn byte_count(&self) -> Result<u64, Error> {
		self.graph.byte_count()
	}

	/// Set the header and optional nonce in the last part of the header
	pub fn set_header_nonce_impl(
		&mut self,
		mut header: Vec<u8>,
		nonce: Option<u32>,
	) -> Result<(), Error> {
		let len = header.len();
		header.truncate(len - mem::size_of::<u32>());
		if let Some(n) = nonce {
			header.write_u32::<LittleEndian>(n)?;
		}
		self.siphash_keys = common::set_header_nonce(header, nonce)?;
		self.graph.reset()?;
		Ok(())
	}

	/// Return siphash masked for type
	fn sipnode(&self, edge: T, uorv: u64) -> Result<T, Error> {
		common::sipnode::<T>(&self.siphash_keys, edge, &self.edge_mask, uorv, false)
	}

	/// Simple implementation of algorithm
	pub fn find_cycles_impl(&mut self) -> Result<Vec<Proof>, Error> {
		for n in 0..self.easiness.to_u64().ok_or(ErrorKind::IntegerCast)? {
			let u = self.sipnode(T::from(n).ok_or(ErrorKind::IntegerCast)?, 0)?;
			let v = self.sipnode(T::from(n).ok_or(ErrorKind::IntegerCast)?, 1)?;
			self.graph.add_edge(
				T::from(u).ok_or(ErrorKind::IntegerCast)?,
				T::from(v).ok_or(ErrorKind::IntegerCast)?,
			)?;
		}
		self.graph.solutions.pop();
		for s in &mut self.graph.solutions {
			s.nonces.sort();
		}
		for s in &self.graph.solutions {
			self.verify_impl(&s)?;
		}
		if self.graph.solutions.len() == 0 {
			Err(ErrorKind::NoSolution)?
		} else {
			Ok(self.graph.solutions.clone())
		}
	}

	/// Verify that given edges are ascending and form a cycle in a header-generated
	/// graph
	pub fn verify_impl(&self, proof: &Proof) -> Result<(), Error> {
		let nonces = &proof.nonces;
		let mut uvs = vec![0u64; 2 * proof.proof_size()];
		let mut xor0: u64 = (self.proof_size as u64 / 2) & 1;
		let mut xor1: u64 = xor0;

		for n in 0..proof.proof_size() {
			if nonces[n] > self.edge_mask.to_u64().ok_or(ErrorKind::IntegerCast)? {
				return Err(ErrorKind::Verification("edge too big".to_owned()))?;
			}
			if n > 0 && nonces[n] <= nonces[n - 1] {
				return Err(ErrorKind::Verification("edges not ascending".to_owned()))?;
			}
			uvs[2 * n] = self
				.sipnode(T::from(nonces[n]).ok_or(ErrorKind::IntegerCast)?, 0)?
				.to_u64()
				.ok_or(ErrorKind::IntegerCast)?;
			uvs[2 * n + 1] = self
				.sipnode(T::from(nonces[n]).ok_or(ErrorKind::IntegerCast)?, 1)?
				.to_u64()
				.ok_or(ErrorKind::IntegerCast)?;
			xor0 ^= uvs[2 * n];
			xor1 ^= uvs[2 * n + 1];
		}
		if xor0 | xor1 != 0 {
			return Err(ErrorKind::Verification(
				"endpoints don't match up".to_owned(),
			))?;
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
						return Err(ErrorKind::Verification("branch in cycle".to_owned()))?;
					}
					j = k;
				}
			}
			if j == i || uvs[j] == uvs[i] {
				return Err(ErrorKind::Verification("cycle dead ends".to_owned()))?;
			}
			i = j ^ 1;
			n += 1;
			if i == 0 {
				break;
			}
		}
		if n == self.proof_size {
			Ok(())
		} else {
			Err(ErrorKind::Verification("cycle too short".to_owned()))?
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn cuckatoo() {
		let ret = basic_solve();
		if let Err(r) = ret {
			panic!("basic_solve: Error: {}", r);
		}
	}

	fn basic_solve() -> Result<(), Error> {
		let easiness_pct = 50;
		let nonce = 1546569;
		let _range = 1;
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
		let mut ctx_u32 =
			CuckatooContext::<u32>::new(edge_bits, proof_size, easiness_pct, max_sols)?;
		let mut bytes = ctx_u32.byte_count()?;
		let mut unit = 0;
		while bytes >= 10240 {
			bytes >>= 10;
			unit += 1;
		}
		println!("Using {}{}B memory", bytes, [' ', 'K', 'M', 'G', 'T'][unit]);
		ctx_u32.set_header_nonce(header, Some(nonce))?;
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
