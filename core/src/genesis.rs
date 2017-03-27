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
use consensus::DEFAULT_SIZESHIFT;
use core::hash::Hashed;
use core::target::Difficulty;

// Genesis block definition. It has no rewards, no inputs, no outputs, no
// fees and a height of zero.
pub fn genesis() -> core::Block {
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
			difficulty: Difficulty::one(),
			total_difficulty: Difficulty::one(),
			utxo_merkle: [].hash(),
			tx_merkle: [].hash(),
			features: core::DEFAULT_BLOCK,
			nonce: 0,
			pow: core::Proof::zero(), // TODO get actual PoW solution
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}
