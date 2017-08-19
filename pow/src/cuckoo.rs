// Copyright 2016 The Grin Developers
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

//! Implementation of Cuckoo Cycle designed by John Tromp. Ported to Rust from
//! the C and Java code at https://github.com/tromp/cuckoo. Note that only the
//! simple miner is included, mostly for testing purposes. John Tromp's Tomato
//! miner will be much faster in almost every environment.

use std::collections::HashSet;
use std::cmp;

use blake2;

use core::core::Proof;
use siphash::siphash24;
use MiningWorker;

const MAXPATHLEN: usize = 8192;

/// A cuckoo-cycle related error
#[derive(Debug)]
pub enum Error {
	/// Unable to find a short enough path
	Path,
	/// Unable to find a solution
	NoSolution,
}

/// An edge in the Cuckoo graph, simply references two u64 nodes.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
struct Edge {
	u: u64,
	v: u64,
}

/// Cuckoo cycle context
pub struct Cuckoo {
	mask: u64,
	size: u64,
	v: [u64; 4],
}

impl Cuckoo {
	/// Initializes a new Cuckoo Cycle setup, using the provided byte array to
	/// generate a seed. In practice for PoW applications the byte array is a
	/// serialized block header.
	pub fn new(header: &[u8], sizeshift: u32) -> Cuckoo {
		let size = 1 << sizeshift;
        let hashed=blake2::blake2b::blake2b(32, &[], header);
        let hashed=hashed.as_bytes();

		let k0 = u8_to_u64(hashed, 0);
		let k1 = u8_to_u64(hashed, 8);
		let mut v = [0; 4];
		v[0] = k0 ^ 0x736f6d6570736575;
		v[1] = k1 ^ 0x646f72616e646f6d;
		v[2] = k0 ^ 0x6c7967656e657261;
		v[3] = k1 ^ 0x7465646279746573;
		Cuckoo {
			v: v,
			size: size,
			mask: (1 << sizeshift) / 2 - 1,
		}
	}

	/// Generates a node in the cuckoo graph generated from our seed. A node is
	/// simply materialized as a u64 from a nonce and an offset (generally 0 or
	/// 1).
	fn new_node(&self, nonce: u64, uorv: u64) -> u64 {
		return ((siphash24(self.v, 2 * nonce + uorv) & self.mask) << 1) | uorv;
	}

	/// Creates a new edge in the cuckoo graph generated by our seed from a
	/// nonce. Generates two node coordinates from the nonce and links them
	/// together.
	fn new_edge(&self, nonce: u64) -> Edge {
		Edge {
			u: self.new_node(nonce, 0),
			v: self.new_node(nonce, 1),
		}
	}

	/// Assuming increasing nonces all smaller than easiness, verifies the
	/// nonces form a cycle in a Cuckoo graph. Each nonce generates an edge, we
	/// build the nodes on both side of that edge and count the connections.
	pub fn verify(&self, proof: Proof, ease: u64) -> bool {
		let easiness = ease * (self.size as u64) / 100;
		let nonces = proof.to_u64s();
		let mut us = vec![0; proof.proof_size];
		let mut vs = vec![0; proof.proof_size];
		for n in 0..proof.proof_size {
			if nonces[n] >= easiness || (n != 0 && nonces[n] <= nonces[n - 1]) {
				return false;
			}
			us[n] = self.new_node(nonces[n], 0);
			vs[n] = self.new_node(nonces[n], 1);
		}
		let mut i = 0;
		let mut count = proof.proof_size;
		loop {
			let mut j = i;
			for k in 0..proof.proof_size {
				// find unique other j with same vs[j]
				if k != i && vs[k] == vs[i] {
					if j != i {
						return false;
					}
					j = k;
				}
			}
			if j == i {
				return false;
			}
			i = j;
			for k in 0..proof.proof_size {
				// find unique other i with same us[i]
				if k != j && us[k] == us[j] {
					if i != j {
						return false;
					}
					i = k;
				}
			}
			if i == j {
				return false;
			}
			count -= 2;
			if i == 0 {
				break;
			}
		}
		count == 0
	}
}

/// Miner for the Cuckoo Cycle algorithm. While the verifier will work for
/// graph sizes up to a u64, the miner is limited to u32 to be more memory
/// compact (so shift <= 32). Non-optimized for now and and so mostly used for
/// tests, being impractical with sizes greater than 2^22.
pub struct Miner {
	easiness: u64,
	proof_size: usize,
	cuckoo: Option<Cuckoo>,
	graph: Vec<u32>,
	sizeshift: u32,
}

impl MiningWorker for Miner {

	/// Creates a new miner
	fn new(ease: u32, 
		   sizeshift: u32,
		   proof_size: usize) -> Miner {
		let size = 1 << sizeshift;
		let graph = vec![0; size + 1];
		let easiness = (ease as u64) * (size as u64) / 100;
		Miner {
			easiness: easiness,
			cuckoo: None,
			graph: graph,
			sizeshift: sizeshift,
			proof_size: proof_size,
		}
	}
	
	fn mine(&mut self, header: &[u8]) -> Result<Proof, Error> {
		let size = 1 << self.sizeshift;
		self.graph = vec![0; size + 1];
		self.cuckoo=Some(Cuckoo::new(header, self.sizeshift));
		self.mine_impl()
	}
}

/// What type of cycle we have found?
enum CycleSol {
	/// A cycle of the right length is a valid proof.
	ValidProof(Vec<u32>),
	/// A cycle of the wrong length is great, but not a proof.
	InvalidCycle(usize),
	/// No cycles have been found.
	NoCycle,
}

impl Miner {
	

	/// Searches for a solution
	pub fn mine_impl(&mut self) -> Result<Proof, Error> {
		let mut us = [0; MAXPATHLEN];
		let mut vs = [0; MAXPATHLEN];
		for nonce in 0..self.easiness {
			us[0] = self.cuckoo.as_mut().unwrap().new_node(nonce, 0) as u32;
			vs[0] = self.cuckoo.as_mut().unwrap().new_node(nonce, 1) as u32;
			let u = self.graph[us[0] as usize];
			let v = self.graph[vs[0] as usize];
			if us[0] == 0 {
				continue; // ignore duplicate edges
			}
			let nu = try!(self.path(u, &mut us)) as usize;
			let nv = try!(self.path(v, &mut vs)) as usize;

			let sol = self.find_sol(nu, &us, nv, &vs);
			match sol {
				CycleSol::ValidProof(res) => {
					return Ok(Proof::new(res.to_vec()));
				},
				CycleSol::InvalidCycle(_) => continue,
				CycleSol::NoCycle => {
					self.update_graph(nu, &us, nv, &vs);
				}
			}
		}
		Err(Error::NoSolution)
	}

	fn path(&self, mut u: u32, us: &mut [u32]) -> Result<u32, Error> {
		let mut nu = 0;
		while u != 0 {
			nu += 1;
			if nu >= MAXPATHLEN {
				while nu != 0 && us[(nu - 1) as usize] != u {
					nu -= 1;
				}
				return Err(Error::Path);
			}
			us[nu as usize] = u;
			u = self.graph[u as usize];
		}
		Ok(nu as u32)
	}

	fn update_graph(&mut self, mut nu: usize, us: &[u32], mut nv: usize, vs: &[u32]) {
		if nu < nv {
			while nu != 0 {
				nu -= 1;
				self.graph[us[nu + 1] as usize] = us[nu];
			}
			self.graph[us[0] as usize] = vs[0];
		} else {
			while nv != 0 {
				nv -= 1;
				self.graph[vs[nv + 1] as usize] = vs[nv];
			}
			self.graph[vs[0] as usize] = us[0];
		}
	}

	fn find_sol(&mut self, mut nu: usize, us: &[u32], mut nv: usize, vs: &[u32]) -> CycleSol {
		if us[nu] == vs[nv] {
			let min = cmp::min(nu, nv);
			nu -= min;
			nv -= min;
			while us[nu] != vs[nv] {
				nu += 1;
				nv += 1;
			}
			if nu + nv + 1 == self.proof_size {
				self.solution(&us, nu as u32, &vs, nv as u32)
			} else {
				CycleSol::InvalidCycle(nu + nv + 1)
			}
		} else {
			CycleSol::NoCycle
		}
	}

	fn solution(&mut self, us: &[u32], mut nu: u32, vs: &[u32], mut nv: u32) -> CycleSol {
		let mut cycle = HashSet::new();
		cycle.insert(Edge {
			u: us[0] as u64,
			v: vs[0] as u64,
		});
		while nu != 0 {
			// u's in even position; v's in odd
			nu -= 1;
			cycle.insert(Edge {
				u: us[((nu + 1) & !1) as usize] as u64,
				v: us[(nu | 1) as usize] as u64,
			});
		}
		while nv != 0 {
			// u's in odd position; v's in even
			nv -= 1;
			cycle.insert(Edge {
				u: vs[(nv | 1) as usize] as u64,
				v: vs[((nv + 1) & !1) as usize] as u64,
			});
		}
		let mut n = 0;
		let mut sol = vec![0; self.proof_size];
		for nonce in 0..self.easiness {
			let edge = self.cuckoo.as_mut().unwrap().new_edge(nonce);
			if cycle.contains(&edge) {
				sol[n] = nonce as u32;
				n += 1;
				cycle.remove(&edge);
			}
		}
		return if n == self.proof_size {
			CycleSol::ValidProof(sol)
		} else {
			CycleSol::NoCycle
		};
	}
}


/// Utility to transform a 8 bytes of a byte array into a u64.
fn u8_to_u64(p:&[u8], i: usize) -> u64 {
	(p[i] as u64) | (p[i + 1] as u64) << 8 | (p[i + 2] as u64) << 16 | (p[i + 3] as u64) << 24 |
	(p[i + 4] as u64) << 32 | (p[i + 5] as u64) << 40 |
	(p[i + 6] as u64) << 48 | (p[i + 7] as u64) << 56
}

#[cfg(test)]
mod test {
	use super::*;
	use core::core::Proof;


	static V1:[u32;42] = [0x1fe9, 0x2050, 0x4581, 0x6322, 0x65ab, 0xb3c1, 0xc1a4, 
			    0xe257, 0x106ae, 0x17b11, 0x202d4, 0x2705d, 0x2deb2, 0x2f80e, 
			 	0x32298, 0x34782, 0x35c5a, 0x37458, 0x38f28, 0x406b2, 0x40e34, 
				0x40fc6, 0x42220, 0x42d13, 0x46c0f, 0x4fd47, 0x55ad2, 0x598f7, 
				0x5aa8f, 0x62aa3, 0x65725, 0x65dcb, 0x671c7, 0x6eb20, 0x752fe, 
				0x7594f, 0x79b9c, 0x7f775, 0x81635, 0x8401c, 0x844e5, 0x89fa8];
	static V2:[u32;42] = [0x2a37, 0x7557, 0xa3c3, 0xfce6, 0x1248e, 0x15837, 0x1827f, 
				0x18a93, 0x1a7dd, 0x1b56b, 0x1ceb4, 0x1f962, 0x1fe2a, 0x29cb9, 
				0x2f30e, 0x2f771, 0x336bf, 0x34355, 0x391d7, 0x39495, 0x3be0c, 
				0x463be, 0x4d0c2, 0x4eead, 0x50214, 0x520de, 0x52a86, 0x53818, 
				0x53b3b, 0x54c0b, 0x572fa, 0x5d79c, 0x5e3c2, 0x6769e, 0x6a0fe, 
				0x6d835, 0x6fc7c, 0x70f03, 0x79d4a, 0x7b03e, 0x81e09, 0x9bd44];
	static V3:[u32;42] = [0x8158, 0x9f18, 0xc4ba, 0x108c7, 0x11caa, 0x13b82, 0x1618f, 
				0x1c83b, 0x1ec89, 0x24354, 0x28864, 0x2a0fb, 0x2ce50, 0x2e8fa, 
				0x32b36, 0x343e6, 0x34dc9, 0x36881, 0x3ffca, 0x40f79, 0x42721, 
				0x43b8c, 0x44b9d, 0x47ed3, 0x4cd34, 0x5278a, 0x5ab64, 0x5b4d4, 
				0x5d842, 0x5fa33, 0x6464e, 0x676ee, 0x685d6, 0x69df0, 0x6a5fd, 
				0x6bda3, 0x72544, 0x77974, 0x7908c, 0x80e67, 0x81ef4, 0x8d882];
	// cuckoo28 at 50% edges of letter 'u'
	static V4:[u32;42] = [0x1CBBFD, 0x2C5452, 0x520338, 0x6740C5, 0x8C6997, 0xC77150, 0xFD4972, 
				0x1060FA7, 0x11BFEA0, 0x1343E8D, 0x14CE02A, 0x1533515, 0x1715E61, 0x1996D9B, 
				0x1CB296B, 0x1FCA180, 0x209A367, 0x20AD02E, 0x23CD2E4, 0x2A3B360, 0x2DD1C0C, 
				0x333A200, 0x33D77BC, 0x3620C78, 0x3DD7FB8, 0x3FBFA49, 0x41BDED2, 0x4A86FD9, 
				0x570DE24, 0x57CAB86, 0x594B886, 0x5C74C94, 0x5DE7572, 0x60ADD6F, 0x635918B, 
				0x6C9E120, 0x6EFA583, 0x7394ACA, 0x7556A23, 0x77F70AA, 0x7CF750A, 0x7F60790];

	/// Find a 42-cycle on Cuckoo20 at 75% easiness and verifiy against a few
	/// known cycle proofs
	/// generated by other implementations.
	#[test]
	fn mine20_vectors() {
		let nonces1 = Miner::new(75, 20, 42).mine(&[49]).unwrap();
		assert_eq!(Proof::new(V1.to_vec()), nonces1);

		let nonces2 = Miner::new(70, 20, 42).mine(&[50]).unwrap();
		assert_eq!(Proof::new(V2.to_vec()), nonces2);

		let nonces3 = Miner::new(70, 20, 42).mine(&[51]).unwrap();
		assert_eq!(Proof::new(V3.to_vec()), nonces3);
	}

	#[test]
	fn validate20_vectors() {
		assert!(Cuckoo::new(&[49], 20).verify(Proof::new(V1.to_vec().clone()), 75));
		assert!(Cuckoo::new(&[50], 20).verify(Proof::new(V2.to_vec().clone()), 70));
		assert!(Cuckoo::new(&[51], 20).verify(Proof::new(V3.to_vec().clone()), 70));
	}

	#[test]
	fn validate28_vectors() {
		let mut test_header=[0;32];
		test_header[0]=24;
		assert!(Cuckoo::new(&test_header, 28).verify(Proof::new(V4.to_vec().clone()), 50));
	}

	#[test]
	fn validate_fail() {
		// edge checks
		assert!(!Cuckoo::new(&[49], 20).verify(Proof::new(vec![0; 42]), 75));
		assert!(!Cuckoo::new(&[49], 20).verify(Proof::new(vec![0xffff; 42]), 75));
		// wrong data for proof
		assert!(!Cuckoo::new(&[50], 20).verify(Proof::new(V1.to_vec().clone()), 75));
		let mut test_header=[0;32];
		test_header[0]=24;
		assert!(!Cuckoo::new(&test_header, 20).verify(Proof::new(V4.to_vec().clone()), 50));
		
	}

	#[test]
	fn mine20_validate() {
		// cuckoo20
		for n in 1..5 {
			let h = [n; 32];
			let nonces = Miner::new(75, 20, 42).mine(&h).unwrap();
			assert!(Cuckoo::new(&h, 20).verify(nonces, 75));
		}
		// cuckoo18
		for n in 1..5 {
			let h = [n; 32];
			let nonces = Miner::new(75, 18, 42).mine(&h).unwrap();
			assert!(Cuckoo::new(&h, 18).verify(nonces, 75));
		}
	}
}
