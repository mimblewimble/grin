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

//! Definition of the genesis block. Placeholder for now.

use time;

use core;
use consensus::{DEFAULT_SIZESHIFT, MAX_TARGET};

use tiny_keccak::Keccak;

// Genesis block definition. It has no rewards, no inputs, no outputs, no
// fees and a height of zero.
pub fn genesis() -> core::Block {
	let mut sha3 = Keccak::new_sha3_256();
	let mut empty_h = [0; 32];
	sha3.update(&[]);
	sha3.finalize(&mut empty_h);

	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 1997 - 1900,
				tm_mon: 7,
				tm_mday: 4,
				..time::empty_tm()
			},
			cuckoo_len: DEFAULT_SIZESHIFT,
			target: MAX_TARGET,
			utxo_merkle: core::hash::Hash::from_vec(empty_h.to_vec()),
			tx_merkle: core::hash::Hash::from_vec(empty_h.to_vec()),
			nonce: 0,
			pow: core::Proof::zero(), // TODO get actual PoW solution
		},
		inputs: vec![],
		outputs: vec![],
		proofs: vec![],
	}
}
