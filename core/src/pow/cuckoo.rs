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

//! Implementation of Cuckoo Cycle designed by John Tromp. Ported to Rust from
//! the C and Java code at https://github.com/tromp/cuckoo. Note that only the
//! simple miner is included, mostly for testing purposes. John Tromp's Tomato
//! miner will be much faster in almost every environment.

use std::collections::HashSet;
use std::cmp;

use blake2;

use core::Proof;
use pow::siphash::siphash24;

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
	pub fn new(header: &[u8], sizeshift: u8) -> Cuckoo {
		let size = 1 << sizeshift;
		let hashed = blake2::blake2b::blake2b(32, &[], header);
		let hashed = hashed.as_bytes();
		Cuckoo {
			v: [
				u8_to_u64(hashed, 0),
				u8_to_u64(hashed, 8),
				u8_to_u64(hashed, 16),
				u8_to_u64(hashed, 24),
			],
			size: size,
			mask: (1 << sizeshift) - 1,
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
	cuckoo: Cuckoo,
	graph: Vec<u32>,
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
	/// Creates a new miner
	pub fn new(header: &[u8], ease: u32, proof_size: usize, sizeshift: u8) -> Miner {
		let cuckoo = Cuckoo::new(header, sizeshift);
		let size = 1 << sizeshift;
		let graph = vec![0; size + 1];
		let easiness = (ease as u64) * (size as u64) / 100;
		Miner {
			easiness: easiness,
			cuckoo: cuckoo,
			graph: graph,
			proof_size: proof_size,
		}
	}

	/// Searches for a solution
	pub fn mine(&mut self) -> Result<Proof, Error> {
		let mut us = [0; MAXPATHLEN];
		let mut vs = [0; MAXPATHLEN];
		for nonce in 0..self.easiness {
			us[0] = self.cuckoo.new_node(nonce, 0) as u32;
			vs[0] = self.cuckoo.new_node(nonce, 1) as u32;
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
				}
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
			let edge = self.cuckoo.new_edge(nonce);
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
fn u8_to_u64(p: &[u8], i: usize) -> u64 {
	(p[i] as u64) | (p[i + 1] as u64) << 8 | (p[i + 2] as u64) << 16 | (p[i + 3] as u64) << 24
		| (p[i + 4] as u64) << 32 | (p[i + 5] as u64) << 40 | (p[i + 6] as u64) << 48
		| (p[i + 7] as u64) << 56
}

#[cfg(test)]
mod test {
	use super::*;
	use core::Proof;

	static V1: [u32; 42] = [
		0x3bbd, 0x4e96, 0x1013b, 0x1172b, 0x1371b, 0x13e6a, 0x1aaa6, 0x1b575, 0x1e237, 0x1ee88,
		0x22f94, 0x24223, 0x25b4f, 0x2e9f3, 0x33b49, 0x34063, 0x3454a, 0x3c081, 0x3d08e, 0x3d863,
		0x4285a, 0x42f22, 0x43122, 0x4b853, 0x4cd0c, 0x4f280, 0x557d5, 0x562cf, 0x58e59, 0x59a62,
		0x5b568, 0x644b9, 0x657e9, 0x66337, 0x6821c, 0x7866f, 0x7e14b, 0x7ec7c, 0x7eed7, 0x80643,
		0x8628c, 0x8949e,
	];
	static V2: [u32; 42] = [
		0x5e3a, 0x8a8b, 0x103d8, 0x1374b, 0x14780, 0x16110, 0x1b571, 0x1c351, 0x1c826, 0x28228,
		0x2909f, 0x29516, 0x2c1c4, 0x334eb, 0x34cdd, 0x38a2c, 0x3ad23, 0x45ac5, 0x46afe, 0x50f43,
		0x51ed6, 0x52ddd, 0x54a82, 0x5a46b, 0x5dbdb, 0x60f6f, 0x60fcd, 0x61c78, 0x63899, 0x64dab,
		0x6affc, 0x6b569, 0x72639, 0x73987, 0x78806, 0x7b98e, 0x7c7d7, 0x7ddd4, 0x7fa88, 0x8277c,
		0x832d9, 0x8ba6f,
	];
	static V3: [u32; 42] = [
		0x308b, 0x9004, 0x91fc, 0x983e, 0x9d67, 0xa293, 0xb4cb, 0xb6c8, 0xccc8, 0xdddc, 0xf04d,
		0x1372f, 0x16ec9, 0x17b61, 0x17d03, 0x1e3bc, 0x1fb0f, 0x29e6e, 0x2a2ca, 0x2a719, 0x3a078,
		0x3b7cc, 0x3c71d, 0x40daa, 0x43e17, 0x46adc, 0x4b359, 0x4c3aa, 0x4ce92, 0x4d06e, 0x51140,
		0x565ac, 0x56b1f, 0x58a8b, 0x5e410, 0x5e607, 0x5ebb5, 0x5f8ae, 0x7aeac, 0x7b902, 0x7d6af,
		0x7f400,
	];
	// cuckoo28 at 50% edges of letter 'u'
	static V4: [u32; 42] = [
		0xf7243, 0x11f130, 0x193812, 0x23b565, 0x279ac3, 0x69b270, 0xe0778f, 0xef51fc, 0x10bf6e8,
		0x13ccf7d, 0x1551177, 0x1b6cfd2, 0x1f872c3, 0x2075681, 0x2e23ccc, 0x2e4c0aa, 0x2f607f1,
		0x3007eeb, 0x3407e9a, 0x35423f9, 0x39e48bf, 0x45e3bf6, 0x46aa484, 0x47c0fe1, 0x4b1d5a6,
		0x4bae0ba, 0x4dfdbaf, 0x5048eda, 0x537da6b, 0x5402887, 0x56b8897, 0x5bd8e8b, 0x622de20,
		0x62be5ce, 0x62d538e, 0x6464518, 0x650a6d5, 0x66ec4fa, 0x66f9476, 0x6b1e5f6, 0x6fd5d88,
		0x701f37b,
	];

	/// Find a 42-cycle on Cuckoo20 at 75% easiness and verifiy against a few
	/// known cycle proofs
	/// generated by other implementations.
	#[test]
	fn mine20_vectors() {
		let nonces1 = Miner::new(&[49], 75, 42, 20).mine().unwrap();
		assert_eq!(Proof::new(V1.to_vec()), nonces1);

		let nonces2 = Miner::new(&[50], 70, 42, 20).mine().unwrap();
		assert_eq!(Proof::new(V2.to_vec()), nonces2);

		let nonces3 = Miner::new(&[51], 70, 42, 20).mine().unwrap();
		assert_eq!(Proof::new(V3.to_vec()), nonces3);
	}

	#[test]
	fn validate20_vectors() {
		assert!(Cuckoo::new(&[49], 20).verify(Proof::new(V1.to_vec().clone()), 75));
		assert!(Cuckoo::new(&[50], 20).verify(Proof::new(V2.to_vec().clone()), 70));
		assert!(Cuckoo::new(&[51], 20).verify(Proof::new(V3.to_vec().clone()), 70));
	}

	/// Just going to disable this for now, as it's painful to try and get a valid
	/// cuckoo28 vector (TBD: 30 is more relevant now anyhow)
	#[test]
	#[ignore]
	fn validate28_vectors() {
		let mut test_header = [0; 32];
		test_header[0] = 24;
		assert!(Cuckoo::new(&test_header, 28).verify(Proof::new(V4.to_vec().clone()), 50));
	}

	#[test]
	fn validate_fail() {
		// edge checks
		assert!(!Cuckoo::new(&[49], 20).verify(Proof::new(vec![0; 42]), 75));
		assert!(!Cuckoo::new(&[49], 20).verify(Proof::new(vec![0xffff; 42]), 75));
		// wrong data for proof
		assert!(!Cuckoo::new(&[50], 20).verify(Proof::new(V1.to_vec().clone()), 75));
		let mut test_header = [0; 32];
		test_header[0] = 24;
		assert!(!Cuckoo::new(&test_header, 20).verify(Proof::new(V4.to_vec().clone()), 50));
	}

	#[test]
	fn mine20_validate() {
		// cuckoo20
		for n in 1..5 {
			let h = [n; 32];
			let nonces = Miner::new(&h, 75, 42, 20).mine().unwrap();
			assert!(Cuckoo::new(&h, 20).verify(nonces, 75));
		}
		// cuckoo18
		for n in 1..5 {
			let h = [n; 32];
			let nonces = Miner::new(&h, 75, 42, 18).mine().unwrap();
			assert!(Cuckoo::new(&h, 18).verify(nonces, 75));
		}
	}
}
