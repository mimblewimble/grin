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

use consensus;
use core;
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

/// Second testnet genesis block (cuckoo30).
pub fn genesis_testnet2() -> core::Block {
	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 2018 - 1900,
				tm_mon: 2,
				tm_mday: 26,
				tm_hour: 16,
				..time::empty_tm()
			},
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			nonce: 1060,
			pow: core::Proof::new(vec![
				0x1940730, 0x333b9d0, 0x4739d6f, 0x4c6cfb1, 0x6e3d6c3, 0x74408a3, 0x7ba2bd2,
				0x83e2024, 0x8ca22b5, 0x9d39ab8, 0xb6646dd, 0xc6698b6, 0xc6f78fe, 0xc99b662,
				0xcf2ae8c, 0xcf41eed, 0xdd073e6, 0xded6af8, 0xf08d1a5, 0x1156a144, 0x11d1160a,
				0x131bb0a5, 0x137ad703, 0x13b0831f, 0x1421683f, 0x147e3c1f, 0x1496fda0, 0x150ba22b,
				0x15cc5bc6, 0x16edf697, 0x17ced40c, 0x17d84f9e, 0x18a515c1, 0x19320d9c, 0x19da4f6d,
				0x1b50bcb1, 0x1b8bc72f, 0x1c7b6964, 0x1d07b3a9, 0x1d189d4d, 0x1d1f9a15, 0x1dafcd41,
			]),
			..Default::default()
		},
		inputs: vec![],
		outputs: vec![],
		kernels: vec![],
	}
}

/// Second testnet genesis block (cuckoo30). Temporary values for now.
pub fn genesis_testnet3() -> core::Block {
	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::hash::Hash([0xff; 32]),
			timestamp: time::Tm {
				tm_year: 2018 - 1900,
				tm_mon: 6,
				tm_mday: 8,
				tm_hour: 18,
				..time::empty_tm()
			},
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			nonce: 4956988373127691,
			pow: core::Proof::new(vec![
					0xa420dc, 0xc8ffee, 0x10e433e, 0x1de9428, 0x2ed4cea, 0x52d907b,
					0x5af0e3f, 0x6b8fcae, 0x8319b53, 0x845ca8c, 0x8d2a13e, 0x8d6e4cc,
					0x9349e8d, 0xa7a33c5, 0xaeac3cb, 0xb193e23, 0xb502e19, 0xb5d9804,
					0xc9ac184, 0xd4f4de3, 0xd7a23b8, 0xf1d8660, 0xf443756, 0x10b833d2,
					0x11418fc5, 0x11b8aeaf, 0x131836ec, 0x132ab818, 0x13a46a55, 0x13df89fe,
					0x145d65b5, 0x166f9c3a, 0x166fe0ef, 0x178cb36f, 0x185baf68, 0x1bbfe563,
					0x1bd637b4, 0x1cfc8382, 0x1d1ed012, 0x1e391ca5, 0x1e999b4c, 0x1f7c6d21,
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
