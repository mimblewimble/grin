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

extern crate env_logger;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate rand;
extern crate time;

use std::fs;
use std::sync::Arc;

use chain::types::*;
use core::core::build;
use core::core::target::Difficulty;
use core::core::transaction;
use core::core::OutputIdentifier;
use core::consensus;
use core::global;
use core::global::ChainTypes;

use keychain::Keychain;

use core::pow;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_coinbase_maturity() {
	let _ = env_logger::init();
	clean_output_dir(".grin");
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let genesis_block = pow::mine_genesis_block().unwrap();

	let chain = chain::Chain::init(
		".grin".to_string(),
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
	).unwrap();

	let prev = chain.head_header().unwrap();

	let keychain = Keychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();
	let key_id4 = keychain.derive_key_id(4).unwrap();

	let mut block =
		core::core::Block::new(&prev, vec![], &keychain, &key_id1, Difficulty::one()).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

	chain.set_txhashset_roots(&mut block, false).unwrap();

	pow::pow_size(
		&mut block.header,
		difficulty,
		global::proofsize(),
		global::sizeshift(),
	).unwrap();

	assert_eq!(block.outputs.len(), 1);
	let coinbase_output = block.outputs[0];
	assert!(
		coinbase_output
			.features
			.contains(transaction::OutputFeatures::COINBASE_OUTPUT)
	);

	let out_id = OutputIdentifier::from_output(&coinbase_output);

	// we will need this later when we want to spend the coinbase output
	let block_hash = block.hash();

	chain
		.process_block(block.clone(), chain::Options::MINE)
		.unwrap();

	let merkle_proof = chain.get_merkle_proof(&out_id, &block.header).unwrap();

	let prev = chain.head_header().unwrap();

	let amount = consensus::REWARD;

	let lock_height = 1 + global::coinbase_maturity();
	assert_eq!(lock_height, 4);

	// here we build a tx that attempts to spend the earlier coinbase output
	// this is not a valid tx as the coinbase output cannot be spent yet
	let coinbase_txn = build::transaction(
		vec![
			build::coinbase_input(amount, block_hash, merkle_proof.clone(), key_id1.clone()),
			build::output(amount - 2, key_id2.clone()),
			build::with_fee(2),
		],
		&keychain,
	).unwrap();

	let mut block = core::core::Block::new(
		&prev,
		vec![&coinbase_txn],
		&keychain,
		&key_id3,
		Difficulty::one(),
	).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

	match chain.set_txhashset_roots(&mut block, false) {
		Err(Error::Transaction(transaction::Error::ImmatureCoinbase)) => (),
		_ => panic!("expected ImmatureCoinbase error here"),
	}

	pow::pow_size(
		&mut block.header,
		difficulty,
		global::proofsize(),
		global::sizeshift(),
	).unwrap();

	// mine enough blocks to increase the height sufficiently for
	// coinbase to reach maturity and be spendable in the next block
	for _ in 0..3 {
		let prev = chain.head_header().unwrap();

		let keychain = Keychain::from_random_seed().unwrap();
		let pk = keychain.derive_key_id(1).unwrap();

		let mut block =
			core::core::Block::new(&prev, vec![], &keychain, &pk, Difficulty::one()).unwrap();
		block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

		let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

		chain.set_txhashset_roots(&mut block, false).unwrap();

		pow::pow_size(
			&mut block.header,
			difficulty,
			global::proofsize(),
			global::sizeshift(),
		).unwrap();

		chain.process_block(block, chain::Options::MINE).unwrap();
	}

	let prev = chain.head_header().unwrap();

	let coinbase_txn = build::transaction(
		vec![
			build::coinbase_input(amount, block_hash, merkle_proof.clone(), key_id1.clone()),
			build::output(amount - 2, key_id2.clone()),
			build::with_fee(2),
		],
		&keychain,
	).unwrap();

	let mut block = core::core::Block::new(
		&prev,
		vec![&coinbase_txn],
		&keychain,
		&key_id4,
		Difficulty::one(),
	).unwrap();

	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

	chain.set_txhashset_roots(&mut block, false).unwrap();

	pow::pow_size(
		&mut block.header,
		difficulty,
		global::proofsize(),
		global::sizeshift(),
	).unwrap();

	let result = chain.process_block(block, chain::Options::MINE);
	match result {
		Ok(_) => (),
		Err(_) => panic!("we did not expect an error here"),
	};
}
