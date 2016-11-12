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
use core::hash::Hashed;

// Genesis block definition. It has no rewards, no inputs, no outputs, no
// fees and a height of zero.
pub fn genesis() -> core::Block {
	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xffu8; 32]),
			timestamp: time::Tm {
				tm_year: 1997,
				tm_mon: 7,
				tm_mday: 4,
				..time::empty_tm()
			},
			td: 0,
			utxo_merkle: [0u8; 32].hash(),
			tx_merkle: [0u8; 32].hash(),
			nonce: 0,
			pow: core::Proof::zero(), // TODO get actual PoW solution
		},
		inputs: vec![],
		outputs: vec![],
		proofs: vec![],
	}
}
