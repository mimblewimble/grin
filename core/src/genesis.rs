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

// required for genesis replacement
//! #![allow(unused_imports)]

use chrono::prelude::{TimeZone, Utc};

use crate::core;
use crate::global;
use crate::pow::{Difficulty, Proof, ProofOfWork};
use crate::util;
use crate::util::secp::constants::SINGLE_BULLET_PROOF_SIZE;
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::secp::Signature;

use crate::core::hash::Hash;
use crate::keychain::BlindingFactor;

/// Genesis block definition for development networks. The proof of work size
/// is small enough to mine it on the fly, so it does not contain its own
/// proof of work solution. Can also be easily mutated for different tests.
pub fn genesis_dev() -> core::Block {
	core::Block::with_header(core::BlockHeader {
		height: 0,
		// previous: core::hash::Hash([0xff; 32]),
		timestamp: Utc.ymd(1997, 8, 4).and_hms(0, 0, 0),
		pow: ProofOfWork {
			nonce: global::get_genesis_nonce(),
			..Default::default()
		},
		..Default::default()
	})
}

/// First testnet genesis block, still subject to change (especially the date,
/// will hopefully come before Christmas).
pub fn genesis_testnet1() -> core::Block {
	core::Block::with_header(core::BlockHeader {
		height: 0,
		timestamp: Utc.ymd(2017, 11, 16).and_hms(20, 0, 0),
		pow: ProofOfWork {
			total_difficulty: Difficulty::min(),
			secondary_scaling: 1,
			nonce: 28205,
			proof: Proof::new(vec![
				0x21e, 0x7a2, 0xeae, 0x144e, 0x1b1c, 0x1fbd, 0x203a, 0x214b, 0x293b, 0x2b74,
				0x2bfa, 0x2c26, 0x32bb, 0x346a, 0x34c7, 0x37c5, 0x4164, 0x42cc, 0x4cc3, 0x55af,
				0x5a70, 0x5b14, 0x5e1c, 0x5f76, 0x6061, 0x60f9, 0x61d7, 0x6318, 0x63a1, 0x63fb,
				0x649b, 0x64e5, 0x65a1, 0x6b69, 0x70f8, 0x71c7, 0x71cd, 0x7492, 0x7b11, 0x7db8,
				0x7f29, 0x7ff8,
			]),
		},
		..Default::default()
	})
}

/// Second testnet genesis block (cuckoo30).
pub fn genesis_testnet2() -> core::Block {
	core::Block::with_header(core::BlockHeader {
		height: 0,
		// previous: core::hash::Hash([0xff; 32]),
		timestamp: Utc.ymd(2018, 3, 26).and_hms(16, 0, 0),
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			secondary_scaling: 1,
			nonce: 1060,
			proof: Proof::new(vec![
				0x1940730, 0x333b9d0, 0x4739d6f, 0x4c6cfb1, 0x6e3d6c3, 0x74408a3, 0x7ba2bd2,
				0x83e2024, 0x8ca22b5, 0x9d39ab8, 0xb6646dd, 0xc6698b6, 0xc6f78fe, 0xc99b662,
				0xcf2ae8c, 0xcf41eed, 0xdd073e6, 0xded6af8, 0xf08d1a5, 0x1156a144, 0x11d1160a,
				0x131bb0a5, 0x137ad703, 0x13b0831f, 0x1421683f, 0x147e3c1f, 0x1496fda0, 0x150ba22b,
				0x15cc5bc6, 0x16edf697, 0x17ced40c, 0x17d84f9e, 0x18a515c1, 0x19320d9c, 0x19da4f6d,
				0x1b50bcb1, 0x1b8bc72f, 0x1c7b6964, 0x1d07b3a9, 0x1d189d4d, 0x1d1f9a15, 0x1dafcd41,
			]),
		},
		..Default::default()
	})
}

/// Second testnet genesis block (cuckoo30). Temporary values for now.
pub fn genesis_testnet3() -> core::Block {
	core::Block::with_header(core::BlockHeader {
		height: 0,
		// previous: core::hash::Hash([0xff; 32]),
		timestamp: Utc.ymd(2018, 7, 8).and_hms(18, 0, 0),
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			secondary_scaling: 1,
			nonce: 4956988373127691,
			proof: Proof::new(vec![
				0xa420dc, 0xc8ffee, 0x10e433e, 0x1de9428, 0x2ed4cea, 0x52d907b, 0x5af0e3f,
				0x6b8fcae, 0x8319b53, 0x845ca8c, 0x8d2a13e, 0x8d6e4cc, 0x9349e8d, 0xa7a33c5,
				0xaeac3cb, 0xb193e23, 0xb502e19, 0xb5d9804, 0xc9ac184, 0xd4f4de3, 0xd7a23b8,
				0xf1d8660, 0xf443756, 0x10b833d2, 0x11418fc5, 0x11b8aeaf, 0x131836ec, 0x132ab818,
				0x13a46a55, 0x13df89fe, 0x145d65b5, 0x166f9c3a, 0x166fe0ef, 0x178cb36f, 0x185baf68,
				0x1bbfe563, 0x1bd637b4, 0x1cfc8382, 0x1d1ed012, 0x1e391ca5, 0x1e999b4c, 0x1f7c6d21,
			]),
		},
		..Default::default()
	})
}

/// 4th testnet genesis block (cuckatoo29 AR, 30+ AF). Temporary values for now (Pow won't verify)
/// NB: Currently set to intenal pre-testnet values
pub fn genesis_testnet4() -> core::Block {
	core::Block::with_header(core::BlockHeader {
		height: 0,
		timestamp: Utc.ymd(2018, 10, 17).and_hms(20, 0, 0),
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(global::initial_block_difficulty()),
			secondary_scaling: global::initial_graph_weight(),
			nonce: 8612241555342799290,
			proof: Proof {
				nonces: vec![
					0x46f3b4, 0x1135f8c, 0x1a1596f, 0x1e10f71, 0x41c03ea, 0x63fe8e7, 0x65af34f,
					0x73c16d3, 0x8216dc3, 0x9bc75d0, 0xae7d9ad, 0xc1cb12b, 0xc65e957, 0xf67a152,
					0xfac6559, 0x100c3d71, 0x11eea08b, 0x1225dfbb, 0x124d61a1, 0x132a14b4,
					0x13f4ec38, 0x1542d236, 0x155f2df0, 0x1577394e, 0x163c3513, 0x19349845,
					0x19d46953, 0x19f65ed4, 0x1a0411b9, 0x1a2fa039, 0x1a72a06c, 0x1b02ddd2,
					0x1b594d59, 0x1b7bffd3, 0x1befe12e, 0x1c82e4cd, 0x1d492478, 0x1de132a5,
					0x1e578b3c, 0x1ed96855, 0x1f222896, 0x1fea0da6,
				],
				edge_bits: 29,
			},
		},
		..Default::default()
	})
}

/// Placeholder for mainnet genesis block, will definitely change before
/// release so no use trying to pre-mine it.
pub fn genesis_main() -> core::Block {
	let gen = core::Block::with_header(core::BlockHeader {
		height: 0,
		timestamp: Utc.ymd(2019, 1, 15).and_hms(12, 0, 0), // REPLACE
		prev_root: Hash::default(),                        // REPLACE
		output_root: Hash::default(),                      // REPLACE
		range_proof_root: Hash::default(),                 // REPLACE
		kernel_root: Hash::default(),                      // REPLACE
		total_kernel_offset: BlindingFactor::zero(),       // REPLACE
		output_mmr_size: 1,
		kernel_mmr_size: 1,
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(10_u64.pow(8)),
			secondary_scaling: 1856,
			nonce: 1, // REPLACE
			proof: Proof {
				nonces: vec![0; 42], // REPLACE
				edge_bits: 29,
			},
		},
		..Default::default()
	});
	let kernel = core::TxKernel {
		features: core::KernelFeatures::COINBASE_KERNEL,
		fee: 0,
		lock_height: 0,
		excess: Commitment::from_vec(vec![]), // REPLACE
		excess_sig: Signature::from_raw_data(&[0; 64]).unwrap(), //REPLACE
	};
	let output = core::Output {
		features: core::OutputFeatures::COINBASE_OUTPUT,
		commit: Commitment::from_vec(vec![]), // REPLACE
		proof: RangeProof {
			plen: SINGLE_BULLET_PROOF_SIZE,
			proof: [0; SINGLE_BULLET_PROOF_SIZE], // REPLACE
		},
	};
	gen.with_reward(output, kernel)
}

#[cfg(test)]
mod test {
	use super::*;

	// TODO hardcode the hashes once genesis is set
	#[test]
	fn mainnet_genesis_hash() {
		let gen_hash = genesis_main().hash();
		println!("mainnet genesis hash: {}", gen_hash.to_hex());
		let gen_bin = core::ser::ser_vec(&genesis_main()).unwrap();
		println!("mainnet genesis full hash: {}\n", gen_bin.hash().to_hex());
		//assert_eq!(gene_hash.to_hex, "");
	}
}
