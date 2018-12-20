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

/// Placeholder for floonet genesis block, will definitely change before
/// release
pub fn genesis_floo() -> core::Block {
	let gen = core::Block::with_header(core::BlockHeader {
		height: 0,
timestamp: Utc.ymd(2018, 12, 20).and_hms(20, 58, 32),
prev_root: Hash::from_hex("ae144568a9ec32faf57e9ca5b5f0997d33f30bd3352fd84c953e6526d847c26b").unwrap(),
output_root: Hash::from_hex("47d2570266451203c62cd003c706e3ec37e9cb4292316744abfa68a1b133bc1c").unwrap(),
range_proof_root: Hash::from_hex("53ea93e80fe37e9a0cbb9c1a1ddf467213922481a4435921aacf55ffb3f388fc").unwrap(),
kernel_root: Hash::from_hex("3ff7655b2846a1313dd72e1c516a2fa262638fabc8e0d4c1dddf80773bbd472d").unwrap(),
total_kernel_offset: BlindingFactor::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
		output_mmr_size: 1,
		kernel_mmr_size: 1,
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(10_u64.pow(6)),
			secondary_scaling: 1856,
nonce: 22,
			proof: Proof {
nonces: vec![48398361, 50434294, 73758991, 93493375, 94716564, 101961133, 153506566, 159476458, 164019912, 208165915, 216747111, 218441011, 221663358, 262514197, 264746362, 278423427, 282069592, 284508695, 297003554, 327321117, 327780367, 329474453, 333639856, 356316379, 366419120, 381872178, 386038638, 389726932, 390055244, 392425788, 399530286, 426997994, 436531599, 456084550, 456375883, 459156409, 474067792, 480904139, 487380747, 489307817, 496780560, 530227836],
				edge_bits: 29,
			},
		},
		..Default::default()
	});
	let kernel = core::TxKernel {
		features: core::KernelFeatures::COINBASE,
		fee: 0,
		lock_height: 0,
excess: Commitment::from_vec(util::from_hex("0817a9e97a070ba5f9fa185c093b4b13b262ed4b4712b6f7c92881b27168f9a2cb".to_string()).unwrap()),
excess_sig: Signature::from_raw_data(&[172, 131, 105, 224, 31, 11, 0, 70, 109, 54, 230, 184, 177, 138, 46, 137, 202, 215, 152, 37, 192, 132, 88, 254, 110, 76, 57, 32, 42, 13, 19, 89, 82, 89, 116, 66, 30, 132, 120, 148, 122, 100, 97, 38, 141, 219, 57, 184, 171, 130, 213, 235, 83, 202, 69, 13, 213, 60, 150, 172, 33, 37, 209, 57]).unwrap(),
	};
	let output = core::Output {
		features: core::OutputFeatures::COINBASE,
commit: Commitment::from_vec(util::from_hex("08f5523cbd8b2e1dae3eefdd9dd1069e0c8a7f055a0611da79f42530c5de0d044b".to_string()).unwrap()),
		proof: RangeProof {
			plen: SINGLE_BULLET_PROOF_SIZE,
proof: [47, 196, 194, 238, 233, 164, 218, 64, 54, 92, 83, 248, 225, 116, 189, 225, 202, 66, 213, 63, 195, 209, 238, 189, 153, 198, 231, 219, 3, 146, 102, 67, 26, 7, 199, 150, 160, 244, 48, 166, 113, 6, 241, 49, 133, 248, 201, 80, 34, 19, 118, 249, 44, 213, 215, 235, 228, 187, 215, 116, 212, 203, 232, 183, 12, 66, 29, 11, 28, 17, 212, 104, 126, 203, 103, 60, 176, 149, 182, 206, 70, 138, 180, 213, 76, 99, 25, 184, 40, 177, 197, 179, 71, 63, 19, 72, 253, 129, 115, 107, 90, 249, 39, 108, 134, 10, 231, 172, 172, 59, 207, 118, 175, 124, 197, 132, 73, 154, 148, 8, 73, 26, 231, 75, 24, 134, 199, 93, 15, 43, 45, 49, 69, 167, 194, 23, 114, 16, 117, 209, 127, 123, 18, 209, 12, 34, 219, 196, 37, 7, 226, 132, 70, 111, 113, 164, 203, 175, 105, 175, 196, 62, 225, 138, 162, 176, 190, 109, 96, 210, 15, 38, 245, 200, 83, 155, 185, 111, 85, 234, 6, 3, 246, 98, 175, 127, 94, 65, 29, 78, 27, 53, 32, 230, 85, 91, 195, 112, 84, 135, 56, 207, 213, 165, 40, 248, 238, 202, 225, 142, 79, 89, 81, 197, 138, 65, 14, 232, 145, 44, 73, 6, 43, 8, 43, 42, 127, 151, 68, 18, 19, 83, 14, 142, 180, 75, 25, 4, 97, 166, 237, 212, 187, 106, 154, 36, 223, 231, 177, 58, 70, 1, 195, 113, 144, 151, 45, 185, 0, 174, 116, 212, 122, 239, 96, 1, 122, 211, 41, 96, 230, 110, 242, 145, 176, 230, 55, 143, 142, 234, 151, 49, 151, 109, 252, 120, 147, 244, 178, 73, 196, 221, 150, 85, 69, 113, 50, 166, 92, 91, 98, 188, 77, 76, 48, 192, 112, 184, 108, 143, 134, 56, 46, 119, 21, 71, 247, 119, 133, 225, 72, 15, 158, 60, 64, 71, 57, 134, 243, 228, 58, 13, 58, 209, 71, 4, 72, 87, 129, 51, 46, 64, 188, 60, 157, 56, 120, 23, 2, 47, 143, 79, 176, 54, 3, 47, 227, 124, 70, 242, 8, 59, 113, 203, 51, 65, 138, 131, 121, 45, 131, 132, 171, 161, 49, 235, 129, 39, 164, 234, 69, 172, 95, 28, 180, 118, 163, 151, 148, 66, 65, 104, 222, 232, 154, 22, 30, 149, 196, 214, 163, 93, 76, 128, 142, 233, 106, 171, 213, 148, 59, 101, 56, 22, 127, 232, 4, 63, 111, 9, 188, 163, 40, 158, 24, 65, 81, 203, 231, 93, 197, 102, 170, 70, 239, 229, 13, 172, 110, 157, 226, 112, 182, 28, 150, 222, 62, 224, 94, 182, 220, 243, 236, 62, 156, 129, 220, 127, 155, 141, 0, 243, 159, 113, 28, 158, 95, 205, 35, 72, 132, 46, 235, 176, 146, 233, 93, 111, 4, 105, 236, 176, 165, 102, 168, 188, 121, 105, 175, 197, 114, 97, 40, 2, 165, 153, 85, 135, 114, 147, 95, 216, 50, 108, 52, 225, 186, 215, 110, 122, 230, 14, 246, 141, 180, 41, 22, 132, 58, 8, 31, 187, 221, 231, 14, 33, 52, 88, 219, 200, 77, 246, 134, 18, 0, 113, 144, 6, 146, 54, 24, 113, 14, 64, 182, 116, 229, 250, 201, 126, 84, 192, 80, 13, 57, 232, 55, 113, 139, 249, 166, 231, 123, 101, 236, 147, 144, 2, 9, 51, 2, 189, 188, 200, 66, 29, 16, 22, 150, 45, 220, 15, 161, 180, 214, 244, 104, 41, 77, 171, 246, 243, 56, 47, 63, 103, 216, 151, 199, 249, 169, 165, 119, 200, 243, 161, 83, 46, 225, 195, 92, 96, 150, 0, 165, 170, 14, 211, 226, 244, 70, 218, 137, 254, 197, 175, 208, 119, 199, 121, 4, 7, 190, 118, 55, 197, 208, 41, 109, 161, 34, 33, 210, 58, 99, 81, 97, 57, 156, 57, 144, 83, 97, 49, 248, 89, 201, 88, 169, 9, 211, 34, 136, 174, 195, 224, 51, 103, 12, 237, 172, 46, 216, 5, 168],
		},
	};
	gen.with_reward(output, kernel)
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
		features: core::KernelFeatures::COINBASE,
		fee: 0,
		lock_height: 0,
		excess: Commitment::from_vec(vec![]), // REPLACE
		excess_sig: Signature::from_raw_data(&[0; 64]).unwrap(), //REPLACE
	};
	let output = core::Output {
		features: core::OutputFeatures::COINBASE,
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
	use crate::ser;
	use crate::core::hash::Hashed;

	// TODO hardcode the hashes once genesis is set
	#[test]
	fn floonet_genesis_hash() {
		let gen_hash = genesis_floo().hash();
		println!("floonet genesis hash: {}", gen_hash.to_hex());
		let gen_bin = ser::ser_vec(&genesis_floo()).unwrap();
		println!("floonet genesis full hash: {}\n", gen_bin.hash().to_hex());
		//assert_eq!(gene_hash.to_hex, "");
	}

	// TODO hardcode the hashes once genesis is set
	#[test]
	fn mainnet_genesis_hash() {
		let gen_hash = genesis_main().hash();
		println!("mainnet genesis hash: {}", gen_hash.to_hex());
		let gen_bin = ser::ser_vec(&genesis_main()).unwrap();
		println!("mainnet genesis full hash: {}\n", gen_bin.hash().to_hex());
		//assert_eq!(gene_hash.to_hex, "");
	}
}
