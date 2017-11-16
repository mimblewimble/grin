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
use consensus;
use core::target::Difficulty;
use global;

/// Genesis block definition for development networks. The proof of work size
/// is small enough to mine it on the fly, so it does not contain its own
/// proof of work solution. Can also be easily mutated for different tests.
pub fn genesis_dev() -> core::Block {
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
			nonce: global::get_genesis_nonce(),
			..Default::default()
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}

/// First testnet genesis block, still subject to change (especially the date,
/// will hopefully come before Christmas).
pub fn genesis_testnet1() -> core::Block {
	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 2017 - 1900,
				tm_mon: 10,
				tm_mday: 16,
				tm_hour: 20,
				..time::empty_tm()
			},
			nonce: 28205,
			pow: core::Proof::new(vec![0x21e, 0x7a2, 0xeae, 0x144e, 0x1b1c, 0x1fbd,
														0x203a, 0x214b, 0x293b, 0x2b74, 0x2bfa, 0x2c26,
														0x32bb, 0x346a, 0x34c7, 0x37c5, 0x4164, 0x42cc,
														0x4cc3, 0x55af, 0x5a70, 0x5b14, 0x5e1c, 0x5f76,
														0x6061, 0x60f9, 0x61d7, 0x6318, 0x63a1, 0x63fb,
														0x649b, 0x64e5, 0x65a1, 0x6b69, 0x70f8, 0x71c7,
														0x71cd, 0x7492, 0x7b11, 0x7db8, 0x7f29, 0x7ff8]),
			..Default::default()
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}

/// Placeholder for mainnet genesis block, will definitely change before
/// release so no use trying to pre-mine it.
pub fn genesis_main() -> core::Block {
	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 2018 - 1900,
				tm_mon: 7,
				tm_mday: 14,
				..time::empty_tm()
			},
			difficulty: Difficulty::from_num(1000),
			total_difficulty: Difficulty::from_num(1000),
			nonce: global::get_genesis_nonce(),
			pow: core::Proof::zero(consensus::PROOFSIZE),
			..Default::default()
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}
