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

//! Definition of the genesis block. Placeholder for now.

// required for genesis replacement
//! #![allow(unused_imports)]

#![cfg_attr(feature = "cargo-clippy", allow(clippy::unreadable_literal))]

use crate::core;
use crate::core::hash::Hash;
use crate::pow::{Difficulty, Proof, ProofOfWork};
use chrono::prelude::{TimeZone, Utc};
use keychain::BlindingFactor;
use util::secp::constants::SINGLE_BULLET_PROOF_SIZE;
use util::secp::pedersen::{Commitment, RangeProof};
use util::secp::Signature;

/// Genesis block definition for development networks. The proof of work size
/// is small enough to mine it on the fly, so it does not contain its own
/// proof of work solution. Can also be easily mutated for different tests.
pub fn genesis_dev() -> core::Block {
	core::Block::with_header(core::BlockHeader {
		height: 0,
		timestamp: Utc.ymd(1997, 8, 4).and_hms(0, 0, 0),
		pow: ProofOfWork {
			nonce: 0,
			..Default::default()
		},
		..Default::default()
	})
}

/// Testnet genesis block
#[allow(clippy::inconsistent_digit_grouping)]
pub fn genesis_test() -> core::Block {
	let gen = core::Block::with_header(core::BlockHeader {
		height: 0,
		timestamp: Utc.ymd(2018, 12, 28).and_hms(20, 48, 4),
		prev_root: Hash::from_hex(
			"00000000000000000017ff4903ef366c8f62e3151ba74e41b8332a126542f538",
		)
		.unwrap(),
		output_root: Hash::from_hex(
			"73b5e0a05ea9e1e4e33b8f1c723bc5c10d17f07042c2af7644f4dbb61f4bc556",
		)
		.unwrap(),
		range_proof_root: Hash::from_hex(
			"667a3ba22f237a875f67c9933037c8564097fa57a3e75be507916de28fc0da26",
		)
		.unwrap(),
		kernel_root: Hash::from_hex(
			"cfdddfe2d938d0026f8b1304442655bbdddde175ff45ddf44cb03bcb0071a72d",
		)
		.unwrap(),
		total_kernel_offset: BlindingFactor::from_hex(
			"0000000000000000000000000000000000000000000000000000000000000000",
		)
		.unwrap(),
		output_mmr_size: 1,
		kernel_mmr_size: 1,
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(10_u64.pow(5)),
			secondary_scaling: 1856,
			nonce: 23,
			proof: Proof {
				nonces: vec![
					16994232_, 22975978_, 32664019_, 44016212_, 50238216_, 57272481_, 85779161_,
					124272202, 125203242, 133907662, 140522149, 145870823, 147481297, 164952795,
					177186722, 183382201, 197418356, 211393794, 239282197, 239323031, 250757611,
					281414565, 305112109, 308151499, 357235186, 374041407, 389924708, 390768911,
					401322239, 401886855, 406986280, 416797005, 418935317, 429007407, 439527429,
					484809502, 486257104, 495589543, 495892390, 525019296, 529899691, 531685572,
				],
				edge_bits: 29,
			},
		},
		..Default::default()
	});
	let kernel = core::TxKernel {
		features: core::KernelFeatures::Coinbase,
		excess: Commitment::from_vec(
			util::from_hex("08df2f1d996cee37715d9ac0a0f3b13aae508d1101945acb8044954aee30960be9")
				.unwrap(),
		),
		excess_sig: Signature::from_raw_data(&[
			25, 176, 52, 246, 172, 1, 12, 220, 247, 111, 73, 101, 13, 16, 157, 130, 110, 196, 123,
			217, 246, 137, 45, 110, 106, 186, 0, 151, 255, 193, 233, 178, 103, 26, 210, 215, 200,
			89, 146, 188, 9, 161, 28, 212, 227, 143, 82, 54, 5, 223, 16, 65, 237, 132, 196, 241,
			39, 76, 133, 45, 252, 131, 88, 0,
		])
		.unwrap(),
	};
	let output = core::Output::new(
		core::OutputFeatures::Coinbase,
		Commitment::from_vec(
			util::from_hex("08c12007af16d1ee55fffe92cef808c77e318dae70c3bc70cb6361f49d517f1b68")
				.unwrap(),
		),
		RangeProof {
			plen: SINGLE_BULLET_PROOF_SIZE,
			proof: [
				159, 156, 202, 179, 128, 169, 14, 227, 176, 79, 118, 180, 62, 164, 2, 234, 123, 30,
				77, 126, 232, 124, 42, 186, 239, 208, 21, 217, 228, 246, 148, 74, 100, 25, 247,
				251, 82, 100, 37, 16, 146, 122, 164, 5, 2, 165, 212, 192, 221, 167, 199, 8, 231,
				149, 158, 216, 194, 200, 62, 15, 53, 200, 188, 207, 0, 79, 211, 88, 194, 211, 54,
				1, 206, 53, 72, 118, 155, 184, 233, 166, 245, 224, 16, 254, 209, 235, 153, 85, 53,
				145, 33, 186, 218, 118, 144, 35, 189, 241, 63, 229, 52, 237, 231, 39, 176, 202, 93,
				247, 85, 131, 16, 193, 247, 180, 33, 138, 255, 102, 190, 213, 129, 174, 182, 167,
				3, 126, 184, 221, 99, 114, 238, 219, 157, 125, 230, 179, 160, 89, 202, 230, 16, 91,
				199, 57, 158, 225, 142, 125, 12, 211, 164, 78, 9, 4, 155, 106, 157, 41, 233, 188,
				237, 205, 184, 53, 0, 190, 24, 215, 42, 44, 184, 120, 58, 196, 198, 190, 114, 50,
				98, 240, 15, 213, 77, 163, 24, 3, 212, 125, 93, 175, 169, 249, 24, 27, 191, 113,
				89, 59, 169, 40, 87, 250, 144, 159, 118, 171, 232, 92, 217, 5, 179, 152, 249, 247,
				71, 239, 26, 180, 82, 177, 226, 132, 185, 3, 33, 162, 120, 98, 87, 109, 57, 100,
				202, 162, 57, 230, 44, 31, 63, 213, 30, 222, 241, 78, 162, 118, 120, 70, 196, 128,
				72, 223, 110, 5, 17, 151, 97, 214, 43, 57, 157, 1, 59, 87, 96, 17, 159, 174, 144,
				217, 159, 87, 36, 113, 41, 155, 186, 252, 162, 46, 22, 80, 133, 3, 113, 248, 11,
				118, 144, 155, 188, 77, 166, 40, 119, 107, 15, 233, 47, 47, 101, 77, 167, 141, 235,
				148, 34, 218, 164, 168, 71, 20, 239, 71, 24, 12, 109, 146, 232, 243, 65, 31, 72,
				186, 131, 190, 43, 227, 157, 41, 49, 126, 136, 51, 41, 50, 213, 37, 186, 223, 87,
				248, 34, 43, 132, 34, 0, 143, 75, 79, 43, 74, 183, 26, 2, 168, 53, 203, 208, 159,
				69, 107, 124, 33, 68, 113, 206, 127, 216, 158, 15, 52, 206, 1, 101, 109, 199, 13,
				131, 122, 29, 131, 133, 125, 219, 70, 69, 144, 133, 68, 233, 67, 203, 132, 160,
				143, 101, 84, 110, 15, 175, 111, 124, 24, 185, 222, 154, 238, 77, 241, 105, 8, 224,
				230, 43, 178, 49, 95, 137, 33, 227, 118, 207, 239, 56, 21, 51, 220, 22, 48, 162,
				22, 118, 229, 215, 248, 112, 198, 126, 180, 27, 161, 237, 56, 2, 220, 129, 126, 11,
				104, 8, 133, 190, 162, 204, 3, 63, 249, 173, 210, 152, 252, 143, 157, 79, 228, 232,
				230, 72, 164, 131, 183, 151, 230, 219, 186, 21, 34, 154, 219, 215, 231, 179, 47,
				217, 44, 115, 203, 157, 35, 195, 113, 235, 194, 102, 96, 205, 24, 221, 213, 147,
				120, 178, 221, 153, 146, 44, 172, 131, 77, 21, 61, 15, 5, 6, 205, 164, 203, 76,
				228, 29, 126, 136, 88, 230, 210, 62, 164, 103, 125, 55, 231, 129, 89, 61, 222, 50,
				71, 71, 75, 230, 70, 80, 85, 193, 136, 183, 222, 146, 46, 235, 0, 222, 118, 32, 70,
				85, 39, 92, 233, 211, 169, 159, 207, 145, 13, 206, 125, 3, 45, 51, 64, 167, 179,
				133, 83, 57, 190, 51, 239, 211, 74, 116, 75, 71, 248, 249, 184, 13, 31, 129, 107,
				104, 179, 76, 194, 186, 4, 13, 122, 167, 254, 126, 153, 50, 8, 1, 200, 203, 213,
				230, 217, 97, 105, 50, 208, 126, 180, 113, 81, 152, 238, 123, 157, 232, 19, 164,
				159, 164, 89, 75, 33, 70, 140, 204, 158, 236, 10, 226, 102, 14, 88, 134, 82, 131,
				36, 195, 127, 158, 81, 252, 223, 165, 11, 52, 105, 245, 245, 228, 235, 168, 175,
				52, 175, 76, 157, 120, 208, 99, 135, 210, 81, 114, 230, 181,
			],
		},
	);
	gen.with_reward(output, kernel)
}

/// Mainnet genesis block
#[allow(clippy::inconsistent_digit_grouping)]
pub fn genesis_main() -> core::Block {
	let gen = core::Block::with_header(core::BlockHeader {
		height: 0,
		timestamp: Utc.ymd(2019, 1, 15).and_hms(16, 1, 26),
		prev_root: Hash::from_hex(
			"0000000000000000002a8bc32f43277fe9c063b9c99ea252b483941dcd06e217",
		)
		.unwrap(),
		output_root: Hash::from_hex(
			"fa7566d275006c6c467876758f2bc87e4cebd2020ae9cf9f294c6217828d6872",
		)
		.unwrap(),
		range_proof_root: Hash::from_hex(
			"1b7fff259aee3edfb5867c4775e4e1717826b843cda6685e5140442ece7bfc2e",
		)
		.unwrap(),
		kernel_root: Hash::from_hex(
			"e8bb096a73cbe6e099968965f5342fc1702ee2802802902286dcf0f279e326bf",
		)
		.unwrap(),
		total_kernel_offset: BlindingFactor::from_hex(
			"0000000000000000000000000000000000000000000000000000000000000000",
		)
		.unwrap(),
		output_mmr_size: 1,
		kernel_mmr_size: 1,
		pow: ProofOfWork {
			total_difficulty: Difficulty::from_num(2_u64.pow(34)),
			secondary_scaling: 1856,
			nonce: 41,
			proof: Proof {
				nonces: vec![
					4391451__, 36730677_, 38198400_, 38797304_, 60700446_, 72910191_, 73050441_,
					110099816, 140885802, 145512513, 149311222, 149994636, 157557529, 160778700,
					162870981, 179649435, 194194460, 227378628, 230933064, 252046196, 272053956,
					277878683, 288331253, 290266880, 293973036, 305315023, 321927758, 353841539,
					356489212, 373843111, 381697287, 389274717, 403108317, 409994705, 411629694,
					431823422, 441976653, 521469643, 521868369, 523044572, 524964447, 530250249,
				],
				edge_bits: 29,
			},
		},
		..Default::default()
	});
	let kernel = core::TxKernel {
		features: core::KernelFeatures::Coinbase,
		excess: Commitment::from_vec(
			util::from_hex("096385d86c5cfda718aa0b7295be0adf7e5ac051edfe130593a2a257f09f78a3b1")
				.unwrap(),
		),
		excess_sig: Signature::from_raw_data(&[
			80, 208, 41, 171, 28, 224, 250, 121, 60, 192, 213, 232, 111, 199, 111, 105, 18, 22, 54,
			165, 107, 33, 186, 113, 186, 100, 12, 42, 72, 106, 42, 20, 67, 253, 188, 178, 228, 246,
			21, 168, 253, 18, 22, 179, 41, 63, 250, 218, 80, 132, 75, 67, 244, 11, 108, 27, 188,
			251, 212, 166, 233, 103, 117, 237,
		])
		.unwrap(),
	};
	let output = core::Output::new(
		core::OutputFeatures::Coinbase,
		Commitment::from_vec(
			util::from_hex("08b7e57c448db5ef25aa119dde2312c64d7ff1b890c416c6dda5ec73cbfed2edea")
				.unwrap(),
		),
		RangeProof {
			plen: SINGLE_BULLET_PROOF_SIZE,
			proof: [
				147, 48, 173, 140, 222, 32, 95, 49, 124, 101, 55, 236, 169, 107, 134, 98, 147, 160,
				72, 150, 21, 169, 162, 119, 180, 211, 165, 151, 200, 115, 84, 76, 130, 71, 73, 50,
				182, 65, 224, 106, 200, 113, 150, 4, 238, 82, 232, 149, 232, 205, 70, 33, 182, 191,
				184, 87, 128, 205, 155, 236, 206, 20, 208, 112, 11, 131, 166, 100, 219, 47, 82,
				162, 108, 66, 95, 215, 119, 173, 136, 148, 76, 223, 255, 56, 4, 58, 39, 147, 237,
				77, 154, 166, 126, 54, 203, 253, 85, 133, 87, 159, 198, 157, 218, 147, 4, 24, 175,
				94, 175, 96, 54, 84, 246, 247, 81, 37, 141, 45, 252, 140, 33, 19, 193, 113, 225,
				48, 243, 30, 193, 230, 204, 226, 167, 24, 228, 53, 41, 143, 206, 93, 100, 255, 225,
				189, 52, 100, 253, 124, 135, 207, 169, 32, 147, 133, 91, 224, 52, 191, 228, 67,
				158, 146, 139, 217, 42, 215, 127, 208, 160, 224, 3, 85, 238, 29, 26, 156, 235, 30,
				208, 196, 8, 220, 253, 186, 140, 88, 62, 117, 152, 220, 112, 10, 170, 159, 145, 67,
				32, 151, 37, 154, 64, 95, 91, 115, 21, 162, 247, 101, 136, 97, 227, 52, 155, 176,
				220, 139, 248, 131, 114, 106, 33, 95, 1, 73, 222, 214, 97, 62, 90, 192, 103, 12,
				12, 82, 2, 36, 125, 124, 39, 200, 167, 208, 59, 219, 3, 201, 207, 84, 85, 70, 63,
				155, 66, 207, 135, 64, 62, 49, 248, 56, 60, 196, 244, 154, 52, 198, 42, 228, 89,
				245, 128, 26, 158, 237, 79, 14, 227, 223, 213, 245, 91, 112, 17, 192, 202, 227,
				147, 196, 116, 171, 214, 248, 199, 150, 91, 155, 95, 255, 49, 4, 221, 78, 57, 84,
				32, 119, 192, 200, 221, 47, 143, 252, 235, 107, 181, 152, 81, 45, 144, 80, 109, 10,
				113, 132, 242, 15, 20, 152, 207, 69, 135, 135, 242, 50, 132, 181, 72, 136, 201,
				190, 65, 109, 16, 63, 118, 4, 6, 53, 122, 22, 182, 216, 65, 163, 3, 213, 201, 91,
				107, 71, 77, 45, 127, 15, 234, 10, 42, 118, 200, 151, 221, 33, 16, 233, 48, 63, 84,
				104, 65, 105, 66, 17, 71, 104, 76, 111, 24, 25, 195, 60, 239, 63, 56, 236, 153, 90,
				80, 132, 80, 192, 44, 209, 135, 47, 128, 101, 253, 238, 114, 49, 9, 193, 139, 29,
				210, 221, 222, 117, 130, 85, 70, 236, 240, 223, 7, 147, 195, 83, 178, 12, 148, 108,
				214, 65, 34, 206, 168, 193, 22, 244, 50, 51, 104, 153, 161, 106, 210, 74, 42, 175,
				203, 143, 144, 14, 9, 161, 20, 113, 53, 252, 242, 165, 76, 191, 129, 219, 48, 138,
				71, 160, 138, 73, 199, 124, 19, 14, 93, 197, 230, 97, 205, 85, 165, 204, 105, 230,
				7, 5, 90, 91, 8, 17, 27, 246, 26, 98, 234, 87, 120, 248, 81, 25, 4, 54, 51, 241,
				202, 184, 199, 86, 215, 86, 197, 163, 72, 81, 2, 74, 195, 17, 165, 150, 177, 205,
				145, 155, 188, 164, 50, 38, 240, 186, 5, 127, 107, 87, 222, 47, 105, 85, 176, 130,
				60, 56, 38, 222, 127, 96, 150, 193, 193, 182, 185, 184, 228, 6, 62, 22, 69, 192,
				191, 243, 47, 128, 86, 26, 170, 149, 157, 151, 18, 15, 188, 46, 205, 157, 43, 226,
				139, 208, 193, 120, 17, 220, 89, 168, 128, 73, 246, 216, 149, 46, 233, 160, 160,
				32, 118, 147, 200, 156, 163, 173, 17, 151, 233, 191, 223, 192, 59, 233, 216, 69,
				174, 168, 214, 99, 150, 146, 23, 227, 180, 148, 206, 233, 230, 82, 188, 159, 135,
				19, 226, 253, 92, 177, 132, 56, 72, 244, 108, 58, 106, 176, 36, 208, 227, 213, 124,
				164, 84, 84, 205, 189, 164, 20, 173, 170, 131, 95, 161, 71, 222, 180, 255, 183, 18,
				156, 243, 168, 216, 103, 38, 160, 20, 71, 148,
			],
		},
	);
	gen.with_reward(output, kernel)
}

#[cfg(test)]
mod test {
	use super::*;
	use crate::core::hash::Hashed;
	use crate::global;
	use crate::ser::{self, ProtocolVersion};
	use util::ToHex;

	#[test]
	fn testnet_genesis_hash() {
		global::set_local_chain_type(global::ChainTypes::Testnet);
		let gen_hash = genesis_test().hash();
		println!("testnet genesis hash: {}", gen_hash.to_hex());
		let gen_bin = ser::ser_vec(&genesis_test(), ProtocolVersion(1)).unwrap();
		println!("testnet genesis full hash: {}\n", gen_bin.hash().to_hex());
		assert_eq!(
			gen_hash.to_hex(),
			"edc758c1370d43e1d733f70f58cf187c3be8242830429b1676b89fd91ccf2dab"
		);
		assert_eq!(
			gen_bin.hash().to_hex(),
			"91c638fc019a54e6652bd6bb3d9c5e0c17e889cef34a5c28528e7eb61a884dc4"
		);
	}

	#[test]
	fn mainnet_genesis_hash() {
		global::set_local_chain_type(global::ChainTypes::Mainnet);
		let gen_hash = genesis_main().hash();
		println!("mainnet genesis hash: {}", gen_hash.to_hex());
		let gen_bin = ser::ser_vec(&genesis_main(), ProtocolVersion(1)).unwrap();
		println!("mainnet genesis full hash: {}\n", gen_bin.hash().to_hex());
		assert_eq!(
			gen_hash.to_hex(),
			"40adad0aec27797b48840aa9e00472015c21baea118ce7a2ff1a82c0f8f5bf82"
		);
		assert_eq!(
			gen_bin.hash().to_hex(),
			"6be6f34b657b785e558e85cc3b8bdb5bcbe8c10e7e58524c8027da7727e189ef"
		);
	}
}
