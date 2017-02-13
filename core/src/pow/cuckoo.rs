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

use crypto::digest::Digest;
use crypto::sha2::Sha256;

use consensus::PROOFSIZE;
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
	pub fn new(header: &[u8], sizeshift: u32) -> Cuckoo {
		let size = 1 << sizeshift;
		let mut hasher = Sha256::new();
		let mut hashed = [0; 32];
		hasher.input(header);
		hasher.result(&mut hashed);

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
		let mut us = [0; PROOFSIZE];
		let mut vs = [0; PROOFSIZE];
		for n in 0..PROOFSIZE {
			if nonces[n] >= easiness || (n != 0 && nonces[n] <= nonces[n - 1]) {
				return false;
			}
			us[n] = self.new_node(nonces[n], 0);
			vs[n] = self.new_node(nonces[n], 1);
		}
		let mut i = 0;
		let mut count = PROOFSIZE;
		loop {
			let mut j = i;
			for k in 0..PROOFSIZE {
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
			for k in 0..PROOFSIZE {
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
	cuckoo: Cuckoo,
	graph: Vec<u32>,
}

/// What type of cycle we have found?
enum CycleSol {
	/// A cycle of the right length is a valid proof.
	ValidProof([u32; PROOFSIZE]),
	/// A cycle of the wrong length is great, but not a proof.
	InvalidCycle(usize),
	/// No cycles have been found.
	NoCycle,
}

impl Miner {
    /// Creates a new miner
	pub fn new(header: &[u8], ease: u32, sizeshift: u32) -> Miner {
		let cuckoo = Cuckoo::new(header, sizeshift);
		let size = 1 << sizeshift;
		let graph = vec![0; size + 1];
		let easiness = (ease as u64) * (size as u64) / 100;
		Miner {
			easiness: easiness,
			cuckoo: cuckoo,
			graph: graph,
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
				CycleSol::ValidProof(res) => return Ok(Proof(res)),
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

	fn find_sol(&self, mut nu: usize, us: &[u32], mut nv: usize, vs: &[u32]) -> CycleSol {
		if us[nu] == vs[nv] {
			let min = cmp::min(nu, nv);
			nu -= min;
			nv -= min;
			while us[nu] != vs[nv] {
				nu += 1;
				nv += 1;
			}
			if nu + nv + 1 == PROOFSIZE {
				self.solution(&us, nu as u32, &vs, nv as u32)
			} else {
				CycleSol::InvalidCycle(nu + nv + 1)
			}
		} else {
			CycleSol::NoCycle
		}
	}

	fn solution(&self, us: &[u32], mut nu: u32, vs: &[u32], mut nv: u32) -> CycleSol {
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
		let mut sol = [0; PROOFSIZE];
		for nonce in 0..self.easiness {
			let edge = self.cuckoo.new_edge(nonce);
			if cycle.contains(&edge) {
				sol[n] = nonce as u32;
				n += 1;
				cycle.remove(&edge);
			}
		}
		return if n == PROOFSIZE {
			CycleSol::ValidProof(sol)
		} else {
			CycleSol::NoCycle
		};
	}
}


/// Utility to transform a 8 bytes of a byte array into a u64.
fn u8_to_u64(p: [u8; 32], i: usize) -> u64 {
	(p[i] as u64) | (p[i + 1] as u64) << 8 | (p[i + 2] as u64) << 16 | (p[i + 3] as u64) << 24 |
	(p[i + 4] as u64) << 32 | (p[i + 5] as u64) << 40 |
	(p[i + 6] as u64) << 48 | (p[i + 7] as u64) << 56
}

#[cfg(test)]
mod test {
	use super::*;
	use core::Proof;

	static V1: Proof = Proof([0xe13, 0x410c, 0x7974, 0x8317, 0xb016, 0xb992, 0xe3c8, 0x1038a,
	                          0x116f0, 0x15ed2, 0x165a2, 0x17793, 0x17dd1, 0x1f885, 0x20932,
	                          0x20936, 0x2171b, 0x28968, 0x2b184, 0x30b8e, 0x31d28, 0x35782,
	                          0x381ea, 0x38321, 0x3b414, 0x3e14b, 0x43615, 0x49a51, 0x4a319,
	                          0x58271, 0x5dbb9, 0x5dbcf, 0x62db4, 0x653d2, 0x655f6, 0x66382,
	                          0x7057d, 0x765b0, 0x79c7c, 0x83167, 0x86e7b, 0x8a5f4]);
	static V2: Proof = Proof([0x33b8, 0x3fd9, 0x8f2b, 0xba0d, 0x11e2d, 0x1d51d, 0x2786e, 0x29625,
	                          0x2a862, 0x2a972, 0x2e6d7, 0x319df, 0x37ce7, 0x3f771, 0x4373b,
	                          0x439b7, 0x48626, 0x49c7d, 0x4a6f1, 0x4a808, 0x4e518, 0x519e3,
	                          0x526bb, 0x54988, 0x564e9, 0x58a6c, 0x5a4dd, 0x63fa2, 0x68ad1,
	                          0x69e52, 0x6bf53, 0x70841, 0x76343, 0x763a4, 0x79681, 0x7d006,
	                          0x7d633, 0x7eebe, 0x7fe7c, 0x811fa, 0x863c1, 0x8b149]);
	static V3: Proof = Proof([0x24ae, 0x5180, 0x9f3d, 0xd379, 0x102c9, 0x15787, 0x16df4, 0x19509,
	                          0x19a78, 0x235a0, 0x24210, 0x24410, 0x2567f, 0x282c3, 0x2d986,
	                          0x2efde, 0x319d7, 0x334d7, 0x336dd, 0x34296, 0x35809, 0x3ad40,
	                          0x46d81, 0x48c92, 0x4b374, 0x4c353, 0x4fe4c, 0x50e4f, 0x53202,
	                          0x5d167, 0x6527c, 0x6a8b5, 0x6c70d, 0x76d90, 0x794f4, 0x7c411,
	                          0x7c5d4, 0x7f59f, 0x7fead, 0x872d8, 0x875b4, 0x95c6b]);
	// cuckoo28 at 50% edges of letter 'u'
	static V4: Proof = Proof([0x1abd16, 0x7bb47e, 0x860253, 0xfad0b2, 0x121aa4d, 0x150a10b,
	                          0x20605cb, 0x20ae7e3, 0x235a9be, 0x2640f4a, 0x2724c36, 0x2a6d38c,
	                          0x2c50b28, 0x30850f2, 0x309668a, 0x30c85bd, 0x345f42c, 0x3901676,
	                          0x432838f, 0x472158a, 0x4d04e9d, 0x4d6a987, 0x4f577bf, 0x4fbc49c,
	                          0x593978d, 0x5acd98f, 0x5e60917, 0x6310602, 0x6385e88, 0x64f149c,
	                          0x66d472e, 0x68e4df9, 0x6b4a89c, 0x6bb751d, 0x6e09792, 0x6e57e1d,
	                          0x6ecfcdd, 0x70abddc, 0x7291dfd, 0x788069e, 0x79a15b1, 0x7d1a1e9]);

	/// Find a 42-cycle on Cuckoo20 at 75% easiness and verifiy against a few
	/// known cycle proofs
	/// generated by other implementations.
	#[test]
	fn mine20_vectors() {
		let nonces1 = Miner::new(&[49], 75, 20).mine().unwrap();
		assert_eq!(V1, nonces1);

		let nonces2 = Miner::new(&[50], 70, 20).mine().unwrap();
		assert_eq!(V2, nonces2);

		let nonces3 = Miner::new(&[51], 70, 20).mine().unwrap();
		assert_eq!(V3, nonces3);
	}

	#[test]
	fn validate20_vectors() {
		assert!(Cuckoo::new(&[49], 20).verify(V1.clone(), 75));
		assert!(Cuckoo::new(&[50], 20).verify(V2.clone(), 70));
		assert!(Cuckoo::new(&[51], 20).verify(V3.clone(), 70));
	}

	#[test]
	fn validate28_vectors() {
		assert!(Cuckoo::new(&[117], 28).verify(V4.clone(), 50));
	}

	#[test]
	fn validate_fail() {
		// edge checks
		assert!(!Cuckoo::new(&[49], 20).verify(Proof([0; 42]), 75));
		assert!(!Cuckoo::new(&[49], 20).verify(Proof([0xffff; 42]), 75));
		// wrong data for proof
		assert!(!Cuckoo::new(&[50], 20).verify(V1.clone(), 75));
		assert!(!Cuckoo::new(&[117], 20).verify(V4.clone(), 50));
	}

	#[test]
	fn mine20_validate() {
		// cuckoo20
		for n in 1..5 {
			let h = [n; 32];
			let nonces = Miner::new(&h, 75, 20).mine().unwrap();
			assert!(Cuckoo::new(&h, 20).verify(nonces, 75));
		}
		// cuckoo18
		for n in 1..5 {
			let h = [n; 32];
			let nonces = Miner::new(&h, 75, 18).mine().unwrap();
			assert!(Cuckoo::new(&h, 18).verify(nonces, 75));
		}
	}
}
