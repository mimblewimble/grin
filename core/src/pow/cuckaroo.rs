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

//! Implementation of Cuckaroo Cycle, based on Cuckoo Cycle designed by
//! John Tromp. Ported to Rust from https://github.com/tromp/cuckoo.
//!
//! Cuckaroo is an ASIC-Resistant variation of Cuckoo (CuckARoo) that's
//! aimed at making the lean mining mode of Cuckoo extremely ineffective.
//! It is one of the 2 proof of works used in Grin (the other one being the
//! more ASIC friendly Cuckatoo).
//!
//! In Cuckaroo, edges are calculated by repeatedly hashing the seeds to
//! obtain blocks of values. Nodes are then extracted from those edges.

use pow::common::{CuckooParams, EdgeType};
use pow::error::{Error, ErrorKind};
use pow::siphash::siphash_block;
use pow::{PoWContext, Proof};

/// Cuckatoo cycle context. Only includes the verifier for now.
pub struct CuckarooContext<T>
where
	T: EdgeType,
{
	params: CuckooParams<T>,
}

impl<T> PoWContext<T> for CuckarooContext<T>
where
	T: EdgeType,
{
	fn new(edge_bits: u8, proof_size: usize, _max_sols: u32) -> Result<Box<Self>, Error> {
		let params = CuckooParams::new(edge_bits, proof_size)?;
		Ok(Box::new(CuckarooContext { params }))
	}

	fn set_header_nonce(
		&mut self,
		header: Vec<u8>,
		nonce: Option<u32>,
		_solve: bool,
	) -> Result<(), Error> {
		self.params.reset_header_nonce(header, nonce)
	}

	fn find_cycles(&mut self) -> Result<Vec<Proof>, Error> {
		unimplemented!()
	}

	fn verify(&self, proof: &Proof) -> Result<(), Error> {
		let nonces = &proof.nonces;
		let mut uvs = vec![0u64; 2 * proof.proof_size()];
		let mut xor0: u64 = 0;
		let mut xor1: u64 = 0;

		for n in 0..proof.proof_size() {
			if nonces[n] > to_u64!(self.params.edge_mask) {
				return Err(ErrorKind::Verification("edge too big".to_owned()))?;
			}
			if n > 0 && nonces[n] <= nonces[n - 1] {
				return Err(ErrorKind::Verification("edges not ascending".to_owned()))?;
			}
			let edge = to_edge!(siphash_block(&self.params.siphash_keys, nonces[n]));
			uvs[2 * n] = to_u64!(edge & self.params.edge_mask);
			uvs[2 * n + 1] = to_u64!((edge >> 32) & self.params.edge_mask);
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
				k = (k + 2) % (2 * self.params.proof_size);
				if k == i {
					break;
				}
				if uvs[k] == uvs[i] {
					// find other edge endpoint matching one at i
					if j != i {
						return Err(ErrorKind::Verification("branch in cycle".to_owned()))?;
					}
					j = k;
				}
			}
			if j == i {
				return Err(ErrorKind::Verification("cycle dead ends".to_owned()))?;
			}
			i = j ^ 1;
			n += 1;
			if i == 0 {
				break;
			}
		}
		if n == self.params.proof_size {
			Ok(())
		} else {
			Err(ErrorKind::Verification("cycle too short".to_owned()))?
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	// empty header, nonce 71
	static V1_19_HASH: [u64; 4] = [
		0x23796193872092ea,
		0xf1017d8a68c4b745,
		0xd312bd53d2cd307b,
		0x840acce5833ddc52,
	];
	static V1_19_SOL: [u64; 42] = [
		0x45e9, 0x6a59, 0xf1ad, 0x10ef7, 0x129e8, 0x13e58, 0x17936, 0x19f7f, 0x208df, 0x23704,
		0x24564, 0x27e64, 0x2b828, 0x2bb41, 0x2ffc0, 0x304c5, 0x31f2a, 0x347de, 0x39686, 0x3ab6c,
		0x429ad, 0x45254, 0x49200, 0x4f8f8, 0x5697f, 0x57ad1, 0x5dd47, 0x607f8, 0x66199, 0x686c7,
		0x6d5f3, 0x6da7a, 0x6dbdf, 0x6f6bf, 0x6ffbb, 0x7580e, 0x78594, 0x785ac, 0x78b1d, 0x7b80d,
		0x7c11c, 0x7da35,
	];

	// empty header, nonce 143
	static V2_19_HASH: [u64; 4] = [
		0x6a54f2a35ab7e976,
		0x68818717ff5cd30e,
		0x9c14260c1bdbaf7,
		0xea5b4cd5d0de3cf0,
	];
	static V2_19_SOL: [u64; 42] = [
		0x2b1e, 0x67d3, 0xb041, 0xb289, 0xc6c3, 0xd31e, 0xd75c, 0x111d7, 0x145aa, 0x1712e, 0x1a3af,
		0x1ecc5, 0x206b1, 0x2a55c, 0x2a9cd, 0x2b67e, 0x321d8, 0x35dde, 0x3721e, 0x37ac0, 0x39edb,
		0x3b80b, 0x3fc79, 0x4148b, 0x42a48, 0x44395, 0x4bbc9, 0x4f775, 0x515c5, 0x56f97, 0x5aa10,
		0x5bc1b, 0x5c56d, 0x5d552, 0x60a2e, 0x66646, 0x6c3aa, 0x70709, 0x71d13, 0x762a3, 0x79d88,
		0x7e3ae,
	];

	#[test]
	fn cuckaroo19_vectors() {
		let mut ctx = CuckarooContext::<u64>::new(19, 42, 0).unwrap();
		ctx.params.siphash_keys = V1_19_HASH.clone();
		assert!(ctx.verify(&Proof::new(V1_19_SOL.to_vec().clone())).is_ok());
		ctx.params.siphash_keys = V2_19_HASH.clone();
		assert!(ctx.verify(&Proof::new(V2_19_SOL.to_vec().clone())).is_ok());
		assert!(ctx.verify(&Proof::zero(42)).is_err());
	}
}
