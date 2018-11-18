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
			let edge = to_edge!(siphash_block(&self.params.siphash_keys, n as u64));
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
