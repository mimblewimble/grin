// Copyright 2021 The Grin Developers
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

//! Implementation of Cuckarood Cycle, based on Cuckoo Cycle designed by
//! John Tromp. Ported to Rust from https://github.com/tromp/cuckoo.
//!
//! Cuckarood is a variation of Cuckaroo that's tweaked at the first HardFork
//! to maintain ASIC-Resistance, as introduced in
//! https://forum.grin.mw/t/mid-july-pow-hardfork-cuckaroo29-cuckarood29
//! It uses a tweaked siphash round in which the rotation by 21 is replaced by
//! a rotation by 25, halves the number of graph nodes in each partition,
//! and requires cycles to alternate between even- and odd-indexed edges.

use crate::global;
use crate::pow::common::CuckooParams;
use crate::pow::error::Error;
use crate::pow::siphash::siphash_block;
use crate::pow::{PoWContext, Proof};

/// Instantiate a new CuckaroodContext as a PowContext. Note that this can't
/// be moved in the PoWContext trait as this particular trait needs to be
/// convertible to an object trait.
pub fn new_cuckarood_ctx(edge_bits: u8, proof_size: usize) -> Result<Box<dyn PoWContext>, Error> {
	let params = CuckooParams::new(edge_bits, edge_bits - 1, proof_size)?;
	Ok(Box::new(CuckaroodContext { params }))
}

/// Cuckarood cycle context. Only includes the verifier for now.
pub struct CuckaroodContext {
	params: CuckooParams,
}

impl PoWContext for CuckaroodContext {
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
		let size = proof.proof_size();
		if size != global::proofsize() {
			return Err(Error::Verification("wrong cycle length".to_owned()));
		}
		let nonces = &proof.nonces;
		let mut uvs = vec![0u64; 2 * size];
		let mut ndir = vec![0usize; 2];
		let mut xor0: u64 = 0;
		let mut xor1: u64 = 0;
		let mask = u64::MAX >> size.leading_zeros(); // round size up to 2-power - 1
											 // the next two arrays form a linked list of nodes with matching bits 4..0|dir
		let mut headu = vec![2 * size; 1 + mask as usize];
		let mut headv = vec![2 * size; 1 + mask as usize];
		let mut prev = vec![0usize; 2 * size];

		for n in 0..size {
			let dir = (nonces[n] & 1) as usize;
			if ndir[dir] >= size / 2 {
				return Err(Error::Verification("edges not balanced".to_owned()));
			}
			if nonces[n] > self.params.edge_mask {
				return Err(Error::Verification("edge too big".to_owned()));
			}
			if n > 0 && nonces[n] <= nonces[n - 1] {
				return Err(Error::Verification("edges not ascending".to_owned()));
			}
			// cuckarood uses a non-standard siphash rotation constant 25 as anti-ASIC tweak
			let edge: u64 = siphash_block(&self.params.siphash_keys, nonces[n], 25, false);
			let idx = 4 * ndir[dir] + 2 * dir;
			let u = edge & self.params.node_mask;
			let v = (edge >> 32) & self.params.node_mask;

			uvs[idx] = u;
			let ubits = ((u << 1 | dir as u64) & mask) as usize;
			prev[idx] = headu[ubits];
			headu[ubits] = idx;

			uvs[idx + 1] = v;
			let vbits = ((v << 1 | dir as u64) & mask) as usize;
			prev[idx + 1] = headv[vbits];
			headv[vbits] = idx + 1;

			xor0 ^= u;
			xor1 ^= v;
			ndir[dir] += 1;
		}
		if xor0 | xor1 != 0 {
			return Err(Error::Verification("endpoints don't match up".to_owned()));
		}
		let mut n = 0;
		let mut i = 0;
		let mut j;
		loop {
			// follow cycle
			j = i;
			let mut k = if i & 1 == 0 {
				headu[((uvs[i] << 1 | 1) & mask) as usize]
			} else {
				headv[((uvs[i] << 1 | 0) & mask) as usize]
			};
			while k != 2 * size {
				if uvs[k] == uvs[i] {
					// find reverse edge endpoint identical to one at i
					if j != i {
						return Err(Error::Verification("branch in cycle".to_owned()));
					}
					j = k;
				}
				k = prev[k];
			}
			if j == i {
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

	// empty header, nonce 64
	static V1_19_HASH: [u64; 4] = [
		0x89f81d7da5e674df,
		0x7586b93105a5fd13,
		0x6fbe212dd4e8c001,
		0x8800c93a8431f938,
	];
	static V1_19_SOL: [u64; 42] = [
		0xa00, 0x3ffb, 0xa474, 0xdc27, 0x182e6, 0x242cc, 0x24de4, 0x270a2, 0x28356, 0x2951f,
		0x2a6ae, 0x2c889, 0x355c7, 0x3863b, 0x3bd7e, 0x3cdbc, 0x3ff95, 0x430b6, 0x4ba1a, 0x4bd7e,
		0x4c59f, 0x4f76d, 0x52064, 0x5378c, 0x540a3, 0x5af6b, 0x5b041, 0x5e9d3, 0x64ec7, 0x6564b,
		0x66763, 0x66899, 0x66e80, 0x68e4e, 0x69133, 0x6b20a, 0x6c2d7, 0x6fd3b, 0x79a8a, 0x79e29,
		0x7ae52, 0x7defe,
	];

	// empty header, nonce 15
	static V2_29_HASH: [u64; 4] = [
		0xe2f917b2d79492ed,
		0xf51088eaaa3a07a0,
		0xaf4d4288d36a4fa8,
		0xc8cdfd30a54e0581,
	];
	static V2_29_SOL: [u64; 42] = [
		0x1a9629, 0x1fb257, 0x5dc22a, 0xf3d0b0, 0x200c474, 0x24bd68f, 0x48ad104, 0x4a17170,
		0x4ca9a41, 0x55f983f, 0x6076c91, 0x6256ffc, 0x63b60a1, 0x7fd5b16, 0x985bff8, 0xaae71f3,
		0xb71f7b4, 0xb989679, 0xc09b7b8, 0xd7601da, 0xd7ab1b6, 0xef1c727, 0xf1e702b, 0xfd6d961,
		0xfdf0007, 0x10248134, 0x114657f6, 0x11f52612, 0x12887251, 0x13596b4b, 0x15e8d831,
		0x16b4c9e5, 0x17097420, 0x1718afca, 0x187fc40c, 0x19359788, 0x1b41d3f1, 0x1bea25a7,
		0x1d28df0f, 0x1ea6c4a0, 0x1f9bf79f, 0x1fa005c6,
	];

	#[test]
	fn cuckarood19_29_vectors() {
		global::set_local_chain_type(global::ChainTypes::Mainnet);
		let mut ctx19 = new_impl(19, 42);
		ctx19.params.siphash_keys = V1_19_HASH;
		assert!(ctx19.verify(&Proof::new(V1_19_SOL.to_vec())).is_ok());
		assert!(ctx19.verify(&Proof::zero(42)).is_err());
		let mut ctx29 = new_impl(29, 42);
		ctx29.params.siphash_keys = V2_29_HASH;
		assert!(ctx29.verify(&Proof::new(V2_29_SOL.to_vec())).is_ok());
		assert!(ctx29.verify(&Proof::zero(42)).is_err());
	}

	fn new_impl(edge_bits: u8, proof_size: usize) -> CuckaroodContext {
		let params = CuckooParams::new(edge_bits, edge_bits - 1, proof_size).unwrap();
		CuckaroodContext { params }
	}
}
