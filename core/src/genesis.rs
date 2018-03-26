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
			pow: core::Proof::new(vec![
				0x21e, 0x7a2, 0xeae, 0x144e, 0x1b1c, 0x1fbd, 0x203a, 0x214b, 0x293b, 0x2b74,
				0x2bfa, 0x2c26, 0x32bb, 0x346a, 0x34c7, 0x37c5, 0x4164, 0x42cc, 0x4cc3, 0x55af,
				0x5a70, 0x5b14, 0x5e1c, 0x5f76, 0x6061, 0x60f9, 0x61d7, 0x6318, 0x63a1, 0x63fb,
				0x649b, 0x64e5, 0x65a1, 0x6b69, 0x70f8, 0x71c7, 0x71cd, 0x7492, 0x7b11, 0x7db8,
				0x7f29, 0x7ff8,
			]),
			..Default::default()
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}

/// Second testnet genesis block (cuckoo30). TBD and don't start getting excited
/// just because you see this reference here... this is for testing mining
/// at cuckoo 30
pub fn genesis_testnet2() -> core::Block {
	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 2018 - 1900,
				tm_mon: 3,
				tm_mday: 26,
				tm_hour: 16,
				..time::empty_tm()
			},
			//TODO: Check this is over-estimated at T2 launch
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			nonce: 329,
			pow: core::Proof::new(vec![
				0x5108b5, 0x6de3c4, 0xded539, 0x15d0e7a, 0x1b05bad, 0x24503fb, 0x420ee7a,
				0x5d905bb, 0x615571f, 0x631d3de, 0x740f1a2, 0x763f189, 0x778edd0, 0x916b09e,
				0x9679a4a, 0x99a7081, 0xad03e98, 0xd0f168c, 0xd878881, 0xda3166f, 0xe402699,
				0x10cac1ec, 0x11598a22, 0x11ad8968, 0x11b7f61a, 0x11c744f5, 0x129911ae,
				0x12d983c3, 0x14052a3c, 0x14464051, 0x16148155, 0x16f8577d, 0x18385648,
				0x18c25a31, 0x1ad7924c, 0x1b6c5079, 0x1c27837c, 0x1ce15e64, 0x1ecd7f12,
				0x1ef421bd, 0x1f5f0642, 0x1f97d069,
			]),
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
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			nonce: global::get_genesis_nonce(),
			pow: core::Proof::zero(consensus::PROOFSIZE),
			..Default::default()
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}
