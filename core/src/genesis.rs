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
use consensus::MINIMUM_DIFFICULTY;
use core::target::Difficulty;
use global;

/// Genesis block definition. It has no rewards, no inputs, no outputs, no
/// fees and a height of zero.
pub fn genesis() -> core::Block {
	let proof_size = global::proofsize();
	let empty_hash = core::hash::Hash([0x0; 32]);
	core::Block {
		header: core::BlockHeader {
			version: 1,
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 1997 - 1900,
				tm_mon: 7,
				tm_mday: 4,
				..time::empty_tm()
			},
			difficulty: Difficulty::from_num(MINIMUM_DIFFICULTY),
			total_difficulty: Difficulty::from_num(MINIMUM_DIFFICULTY),
			utxo_root: empty_hash,
			range_proof_root: empty_hash,
			kernel_root: empty_hash,
			nonce: global::get_genesis_nonce(),
			pow: core::Proof::zero(proof_size), // TODO get actual PoW solution
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}
