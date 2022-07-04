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

//! Implementation of Cuckaroom Cycle, based on Cuckoo Cycle designed by
//! John Tromp. Ported to Rust from https://github.com/tromp/cuckoo.
//!
//! Cuckaroom is a variation of Cuckaroo that's tweaked at the second HardFork
//! to maintain ASIC-Resistance.
//! It uses a tweaked edge block generation where states are xored with all later
//! states, reverts to standard siphash, and most importantly, identifies cycles
//! in a mono-partite graph, from which it derives the letter 'm'.

use crate::global;
use crate::pow::common::CuckooParams;
use crate::pow::error::Error;
use crate::pow::siphash::siphash_block;
use crate::pow::{PoWContext, Proof};

/// Instantiate a new CuckaroomContext as a PowContext. Note that this can't
/// be moved in the PoWContext trait as this particular trait needs to be
/// convertible to an object trait.
pub fn new_cuckaroom_ctx(edge_bits: u8, proof_size: usize) -> Result<Box<dyn PoWContext>, Error> {
	let params = CuckooParams::new(edge_bits, edge_bits, proof_size)?;
	Ok(Box::new(CuckaroomContext { params }))
}

/// Cuckaroom cycle context. Only includes the verifier for now.
pub struct CuckaroomContext {
	params: CuckooParams,
}

impl PoWContext for CuckaroomContext {
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
		let mut from = vec![0u64; size];
		let mut to = vec![0u64; size];
		let mut xor_from: u64 = 0;
		let mut xor_to: u64 = 0;
		let mask = u64::MAX >> size.leading_zeros(); // round size up to 2-power - 1
											 // the next two arrays form a linked list of nodes with matching bits 6..1
		let mut head = vec![size; 1 + mask as usize];
		let mut prev = vec![0usize; size];

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
			from[n] = u;
			let bits = (u & mask) as usize;
			prev[n] = head[bits];
			head[bits] = n;
			to[n] = v;
			xor_from ^= from[n];
			xor_to ^= to[n];
		}
		if xor_from != xor_to {
			return Err(Error::Verification("endpoints don't match up".to_owned()));
		}
		let mut visited = vec![false; size];
		let mut n = 0;
		let mut i = 0;
		loop {
			// follow cycle
			if visited[i] {
				return Err(Error::Verification("branch in cycle".to_owned()));
			}
			visited[i] = true;
			let mut k = head[(to[i] & mask) as usize];
			loop {
				if k == size {
					return Err(Error::Verification("cycle dead ends".to_owned()));
				}
				if from[k] == to[i] {
					break;
				}
				k = prev[k];
			}
			i = k;
			n += 1;
			if i == 0 {
				// must cycle back to start or find branch
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
		0xdb7896f799c76dab,
		0x352e8bf25df7a723,
		0xf0aa29cbb1150ea6,
		0x3206c2759f41cbd5,
	];
	static V1_19_SOL: [u64; 42] = [
		0x0413c, 0x05121, 0x0546e, 0x1293a, 0x1dd27, 0x1e13e, 0x1e1d2, 0x22870, 0x24642, 0x24833,
		0x29190, 0x2a732, 0x2ccf6, 0x302cf, 0x32d9a, 0x33700, 0x33a20, 0x351d9, 0x3554b, 0x35a70,
		0x376c1, 0x398c6, 0x3f404, 0x3ff0c, 0x48b26, 0x49a03, 0x4c555, 0x4dcda, 0x4dfcd, 0x4fbb6,
		0x50275, 0x584a8, 0x5da0d, 0x5dbf1, 0x6038f, 0x66540, 0x72bbd, 0x77323, 0x77424, 0x77a14,
		0x77dc9, 0x7d9dc,
	];

	// empty header, nonce 15
	static V2_29_HASH: [u64; 4] = [
		0xe4b4a751f2eac47d,
		0x3115d47edfb69267,
		0x87de84146d9d609e,
		0x7deb20eab6d976a1,
	];
	static V2_29_SOL: [u64; 42] = [
		0x04acd28, 0x29ccf71, 0x2a5572b, 0x2f31c2c, 0x2f60c37, 0x317fe1d, 0x32f6d4c, 0x3f51227,
		0x45ee1dc, 0x535eeb8, 0x5e135d5, 0x6184e3d, 0x6b1b8e0, 0x6f857a9, 0x8916a0f, 0x9beb5f8,
		0xa3c8dc9, 0xa886d94, 0xaab6a57, 0xd6df8f8, 0xe4d630f, 0xe6ae422, 0xea2d658, 0xf7f369b,
		0x10c465d8, 0x1130471e, 0x12049efb, 0x12f43bc5, 0x15b493a6, 0x16899354, 0x1915dfca,
		0x195c3dac, 0x19b09ab6, 0x1a1a8ed7, 0x1bba748f, 0x1bdbf777, 0x1c806542, 0x1d201b53,
		0x1d9e6af7, 0x1e99885e, 0x1f255834, 0x1f9c383b,
	];

	#[test]
	fn cuckaroom19_29_vectors() {
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

	fn new_impl(edge_bits: u8, proof_size: usize) -> CuckaroomContext {
		let params = CuckooParams::new(edge_bits, edge_bits, proof_size).unwrap();
		CuckaroomContext { params }
	}
}
