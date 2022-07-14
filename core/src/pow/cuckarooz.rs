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

//! Implementation of Cuckarooz Cycle, based on Cuckoo Cycle designed by
//! John Tromp. Ported to Rust from https://github.com/tromp/cuckoo.
//!
//! Cuckarooz is a variation of Cuckaroo that's tweaked at the third HardFork
//! to maintain ASIC-Resistance, as introduced in
//! https://forum.grin.mw/t/introducing-the-final-tweak-cuckarooz
//! It completes the choices of undirected vs directed edges and bipartite vs
//! monopartite graphs, and is named after the last letter of the alphabet
//! accordingly.

use crate::global;
use crate::pow::common::CuckooParams;
use crate::pow::error::Error;
use crate::pow::siphash::siphash_block;
use crate::pow::{PoWContext, Proof};

/// Instantiate a new CuckaroozContext as a PowContext. Note that this can't
/// be moved in the PoWContext trait as this particular trait needs to be
/// convertible to an object trait.
pub fn new_cuckarooz_ctx(edge_bits: u8, proof_size: usize) -> Result<Box<dyn PoWContext>, Error> {
	let params = CuckooParams::new(edge_bits, edge_bits + 1, proof_size)?;
	Ok(Box::new(CuckaroozContext { params }))
}

/// Cuckarooz cycle context. Only includes the verifier for now.
pub struct CuckaroozContext {
	params: CuckooParams,
}

impl PoWContext for CuckaroozContext {
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
		let mut xoruv: u64 = 0;
		let mask = u64::MAX >> size.leading_zeros(); // round size up to 2-power - 1
											 // the next two arrays form a linked list of nodes with matching bits 6..1
		let mut head = vec![2 * size; 1 + mask as usize];
		let mut prev = vec![0usize; 2 * size];

		for n in 0..size {
			if nonces[n] > self.params.edge_mask {
				return Err(Error::Verification("edge too big".to_owned()));
			}
			if n > 0 && nonces[n] <= nonces[n - 1] {
				return Err(Error::Verification("edges not ascending".to_owned()));
			}
			// 21 is standard siphash rotation constant
			let edge: u64 = siphash_block(&self.params.siphash_keys, nonces[n], 21, true);
			let u = edge & self.params.node_mask;
			let v = (edge >> 32) & self.params.node_mask;

			uvs[2 * n] = u;
			let bits = (u & mask) as usize;
			prev[2 * n] = head[bits];
			head[bits] = 2 * n;

			uvs[2 * n + 1] = v;
			let bits = (v & mask) as usize;
			prev[2 * n + 1] = head[bits];
			head[bits] = 2 * n + 1;

			xoruv ^= uvs[2 * n] ^ uvs[2 * n + 1];
		}
		if xoruv != 0 {
			return Err(Error::Verification("endpoints don't match up".to_owned()));
		}
		// make prev lists circular
		for n in 0..(2 * size) {
			if prev[n] == 2 * size {
				let bits = (uvs[n] & mask) as usize;
				prev[n] = head[bits];
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
				if uvs[k] == uvs[i] {
					// find other edge endpoint matching one at i
					if j != i {
						return Err(Error::Verification("branch in cycle".to_owned()));
					}
					j = k;
				}
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
		if n == self.params.proof_size {
			Ok(())
		} else {
			Err(Error::Verification("cycle too short".to_owned()))
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	// empty header, nonce 71
	static V1_19_HASH: [u64; 4] = [
		0xd129f63fba4d9a85,
		0x457dcb3666c5e09c,
		0x045247a2e2ee75f7,
		0x1a0f2e1bcb9d93ff,
	];
	static V1_19_SOL: [u64; 42] = [
		0x33b6, 0x487b, 0x88b7, 0x10bf6, 0x15144, 0x17cb7, 0x22621, 0x2358e, 0x23775, 0x24fb3,
		0x26b8a, 0x2876c, 0x2973e, 0x2f4ba, 0x30a62, 0x3a36b, 0x3ba5d, 0x3be67, 0x3ec56, 0x43141,
		0x4b9c5, 0x4fa06, 0x51a5c, 0x523e5, 0x53d08, 0x57d34, 0x5c2de, 0x60bba, 0x62509, 0x64d69,
		0x6803f, 0x68af4, 0x6bd52, 0x6f041, 0x6f900, 0x70051, 0x7097d, 0x735e8, 0x742c2, 0x79ae5,
		0x7f64d, 0x7fd49,
	];

	// empty header, nonce 15
	static V2_29_HASH: [u64; 4] = [
		0x34bb4c75c929a2f5,
		0x21df13263aa81235,
		0x37d00939eae4be06,
		0x473251cbf6941553,
	];
	static V2_29_SOL: [u64; 42] = [
		0x49733a, 0x1d49107, 0x253d2ca, 0x5ad5e59, 0x5b671bd, 0x5dcae1c, 0x5f9a589, 0x65e9afc,
		0x6a59a45, 0x7d9c6d3, 0x7df96e4, 0x8b26174, 0xa17b430, 0xa1c8c0d, 0xa8a0327, 0xabd7402,
		0xacb7c77, 0xb67524f, 0xc1c15a6, 0xc7e2c26, 0xc7f5d8d, 0xcae478a, 0xdea9229, 0xe1ab49e,
		0xf57c7db, 0xfb4e8c5, 0xff314aa, 0x110ccc12, 0x143e546f, 0x17007af8, 0x17140ea2,
		0x173d7c5d, 0x175cd13f, 0x178b8880, 0x1801edc5, 0x18c8f56b, 0x18c8fe6d, 0x19f1a31a,
		0x1bb028d1, 0x1caaa65a, 0x1cf29bc2, 0x1dbde27d,
	];

	#[test]
	fn cuckarooz19_29_vectors() {
		global::set_local_chain_type(global::ChainTypes::Mainnet);
		let mut ctx19 = new_impl(19, 42);
		ctx19.params.siphash_keys = V1_19_HASH.clone();
		assert!(ctx19
			.verify(&Proof::new(V1_19_SOL.to_vec().clone()))
			.is_ok());
		assert!(ctx19.verify(&Proof::zero(42)).is_err());
		let mut ctx29 = new_impl(29, 42);
		ctx29.params.siphash_keys = V2_29_HASH.clone();
		assert!(ctx29
			.verify(&Proof::new(V2_29_SOL.to_vec().clone()))
			.is_ok());
		assert!(ctx29.verify(&Proof::zero(42)).is_err());
	}

	fn new_impl(edge_bits: u8, proof_size: usize) -> CuckaroozContext {
		let params = CuckooParams::new(edge_bits, edge_bits + 1, proof_size).unwrap();
		CuckaroozContext { params }
	}
}
