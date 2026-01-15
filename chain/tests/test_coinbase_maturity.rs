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

use self::chain::types::NoopAdapter;
use self::chain::Error;
use self::core::core::KernelFeatures;
use self::core::global::{self, ChainTypes};
use self::core::libtx::{self, build, ProofBuilder};
use self::core::{consensus, pow};
use self::keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::fs;
use std::sync::Arc;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_coinbase_maturity() {
	util::init_test_logger();
	let chain_dir = ".grin_coinbase";
	clean_output_dir(chain_dir);
	global::set_local_chain_type(ChainTypes::AutomatedTesting);

	let genesis_block = pow::mine_genesis_block().unwrap();

	{
		let chain = chain::Chain::init(
			chain_dir.to_string(),
			Arc::new(NoopAdapter {}),
			genesis_block,
			pow::verify_size,
			false,
		)
		.unwrap();

		let prev = chain.head_header().unwrap();

		let keychain = ExtKeychain::from_random_seed(false).unwrap();
		let builder = ProofBuilder::new(&keychain);
		let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
		let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
		let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();
		let key_id4 = ExtKeychainPath::new(1, 4, 0, 0, 0).to_identifier();

		let next_header_info =
			consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
		let reward = libtx::reward::output(&keychain, &builder, &key_id1, 0, false).unwrap();
		let mut block =
			core::core::Block::new(&prev, &[], next_header_info.difficulty, reward).unwrap();
		block.header.timestamp = prev.timestamp + Duration::seconds(60);
		block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		chain.set_txhashset_roots(&mut block).unwrap();

		pow::pow_size(
			&mut block.header,
			next_header_info.difficulty,
			global::proofsize(),
			global::min_edge_bits(),
		)
		.unwrap();

		assert_eq!(block.outputs().len(), 1);
		let coinbase_output = block.outputs()[0];
		assert!(coinbase_output.is_coinbase());

		chain
			.process_block(block.clone(), chain::Options::MINE)
			.unwrap();

		let prev = chain.head_header().unwrap();

		let amount = consensus::REWARD;

		let lock_height = 1 + global::coinbase_maturity();
		assert_eq!(lock_height, 4);

		// here we build a tx that attempts to spend the earlier coinbase output
		// this is not a valid tx as the coinbase output cannot be spent yet
		let coinbase_txn = build::transaction(
			KernelFeatures::Plain { fee: 2.into() },
			&[
				build::coinbase_input(amount, key_id1.clone()),
				build::output(amount - 2, key_id2.clone()),
			],
			&keychain,
			&builder,
		)
		.unwrap();

		let txs = &[coinbase_txn.clone()];
		let fees = txs.iter().map(|tx| tx.fee()).sum();
		let reward = libtx::reward::output(&keychain, &builder, &key_id3, fees, false).unwrap();
		let next_header_info =
			consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
		let mut block =
			core::core::Block::new(&prev, txs, next_header_info.difficulty, reward).unwrap();
		block.header.timestamp = prev.timestamp + Duration::seconds(60);
		block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		chain.set_txhashset_roots(&mut block).unwrap();

		// Confirm the tx attempting to spend the coinbase output
		// is not valid at the current block height given the current chain state.
		match chain.verify_coinbase_maturity(&coinbase_txn.inputs()) {
			Ok(_) => {}
			Err(e) => match e {
				Error::ImmatureCoinbase => {}
				_ => panic!("Expected transaction error with immature coinbase."),
			},
		}

		pow::pow_size(
			&mut block.header,
			next_header_info.difficulty,
			global::proofsize(),
			global::min_edge_bits(),
		)
		.unwrap();

		// mine enough blocks to increase the height sufficiently for
		// coinbase to reach maturity and be spendable in the next block
		for _ in 0..3 {
			let prev = chain.head_header().unwrap();

			let keychain = ExtKeychain::from_random_seed(false).unwrap();
			let builder = ProofBuilder::new(&keychain);
			let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();

			let next_header_info =
				consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
			let reward = libtx::reward::output(&keychain, &builder, &key_id1, 0, false).unwrap();
			let mut block =
				core::core::Block::new(&prev, &[], next_header_info.difficulty, reward).unwrap();

			block.header.timestamp = prev.timestamp + Duration::seconds(60);
			block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

			chain.set_txhashset_roots(&mut block).unwrap();

			pow::pow_size(
				&mut block.header,
				next_header_info.difficulty,
				global::proofsize(),
				global::min_edge_bits(),
			)
			.unwrap();

			assert_eq!(block.outputs().len(), 1);
			let coinbase_output = block.outputs()[0];
			assert!(coinbase_output.is_coinbase());

			chain
				.process_block(block.clone(), chain::Options::MINE)
				.unwrap();

			let prev = chain.head_header().unwrap();

			let amount = consensus::REWARD;

			let lock_height = 1 + global::coinbase_maturity();
			assert_eq!(lock_height, 4);

			// here we build a tx that attempts to spend the earlier coinbase output
			// this is not a valid tx as the coinbase output cannot be spent yet
			let coinbase_txn = build::transaction(
				KernelFeatures::Plain { fee: 2.into() },
				&[
					build::coinbase_input(amount, key_id1.clone()),
					build::output(amount - 2, key_id2.clone()),
				],
				&keychain,
				&builder,
			)
			.unwrap();

			let txs = &[coinbase_txn.clone()];
			let fees = txs.iter().map(|tx| tx.fee()).sum();
			let reward = libtx::reward::output(&keychain, &builder, &key_id3, fees, false).unwrap();
			let next_header_info =
				consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
			let mut block =
				core::core::Block::new(&prev, txs, next_header_info.difficulty, reward).unwrap();
			block.header.timestamp = prev.timestamp + Duration::seconds(60);
			block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

			chain.set_txhashset_roots(&mut block).unwrap();

			// Confirm the tx attempting to spend the coinbase output
			// is not valid at the current block height given the current chain state.
			match chain.verify_coinbase_maturity(&coinbase_txn.inputs()) {
				Ok(_) => {}
				Err(e) => match e {
					Error::ImmatureCoinbase => {}
					_ => panic!("Expected transaction error with immature coinbase."),
				},
			}

			pow::pow_size(
				&mut block.header,
				next_header_info.difficulty,
				global::proofsize(),
				global::min_edge_bits(),
			)
			.unwrap();

			// mine enough blocks to increase the height sufficiently for
			// coinbase to reach maturity and be spendable in the next block
			for _ in 0..3 {
				let prev = chain.head_header().unwrap();

				let keychain = ExtKeychain::from_random_seed(false).unwrap();
				let builder = ProofBuilder::new(&keychain);
				let pk = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();

				let reward = libtx::reward::output(&keychain, &builder, &pk, 0, false).unwrap();
				let next_header_info =
					consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
				let mut block =
					core::core::Block::new(&prev, &[], next_header_info.difficulty, reward)
						.unwrap();
				block.header.timestamp = prev.timestamp + Duration::seconds(60);
				block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

				chain.set_txhashset_roots(&mut block).unwrap();

				pow::pow_size(
					&mut block.header,
					next_header_info.difficulty,
					global::proofsize(),
					global::min_edge_bits(),
				)
				.unwrap();

				chain.process_block(block, chain::Options::MINE).unwrap();
			}

			let prev = chain.head_header().unwrap();

			// Confirm the tx spending the coinbase output is now valid.
			// The coinbase output has matured sufficiently based on current chain state.
			chain
				.verify_coinbase_maturity(&coinbase_txn.inputs())
				.unwrap();

			let txs = &[coinbase_txn];
			let fees = txs.iter().map(|tx| tx.fee()).sum();
			let next_header_info =
				consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
			let reward = libtx::reward::output(&keychain, &builder, &key_id4, fees, false).unwrap();
			let mut block =
				core::core::Block::new(&prev, txs, next_header_info.difficulty, reward).unwrap();

			block.header.timestamp = prev.timestamp + Duration::seconds(60);
			block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

			chain.set_txhashset_roots(&mut block).unwrap();

			pow::pow_size(
				&mut block.header,
				next_header_info.difficulty,
				global::proofsize(),
				global::min_edge_bits(),
			)
			.unwrap();

			let result = chain.process_block(block, chain::Options::MINE);
			match result {
				Ok(_) => (),
				Err(_) => panic!("we did not expect an error here"),
			};
		}
	}
	// Cleanup chain directory
	clean_output_dir(chain_dir);
}
