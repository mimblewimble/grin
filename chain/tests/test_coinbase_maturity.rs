// Copyright 2017 The Grin Developers
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
extern crate grin_pow as pow;
extern crate rand;
extern crate time;

use std::fs;
use std::sync::Arc;

use chain::types::*;
use core::core::build;
use core::core::target::Difficulty;
use core::core::transaction;
use core::consensus;
use core::global;
use core::global::ChainTypes;

use keychain::Keychain;

use pow::{cuckoo, types, MiningWorker};

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_coinbase_maturity() {
	let _ = env_logger::init();
	clean_output_dir(".grin");
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let genesis_block = pow::mine_genesis_block(None).unwrap();

	let chain = chain::Chain::init(
		".grin".to_string(),
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
	).unwrap();

	let mut miner_config = types::MinerConfig {
		enable_mining: true,
		burn_reward: true,
		..Default::default()
	};
	miner_config.cuckoo_miner_plugin_dir = Some(String::from("../target/debug/deps"));

	let mut cuckoo_miner = cuckoo::Miner::new(
		consensus::EASINESS,
		global::sizeshift() as u32,
		global::proofsize(),
	);

	let prev = chain.head_header().unwrap();

	let keychain = Keychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();
	let key_id4 = keychain.derive_key_id(4).unwrap();

	let mut block = core::core::Block::new(
		&prev,
		vec![],
		&keychain,
		&key_id1,
		Difficulty::one()
	).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	block.header.difficulty = difficulty.clone();
	chain.set_sumtree_roots(&mut block, false).unwrap();

	pow::pow_size(
		&mut cuckoo_miner,
		&mut block.header,
		difficulty,
		global::sizeshift() as u32,
	).unwrap();

	assert_eq!(block.outputs.len(), 1);
	assert!(
		block.outputs[0]
			.features
			.contains(transaction::COINBASE_OUTPUT)
	);

	// we will need this later when we want to spend the coinbase output
	let block_hash = block.hash();

	chain.process_block(block, chain::MINE).unwrap();

	let prev = chain.head_header().unwrap();

	let amount = consensus::REWARD;

	let lock_height = 1 + global::coinbase_maturity();
	assert_eq!(lock_height, 4);

	// here we build a tx that attempts to spend the earlier coinbase output
	// this is not a valid tx as the coinbase output cannot be spent yet
	let (coinbase_txn, _) = build::transaction(
		vec![
			build::coinbase_input(amount, block_hash, key_id1.clone()),
			build::output(amount - 2, key_id2.clone()),
			build::with_fee(2),
		],
		&keychain,
	).unwrap();

	let mut block =
		core::core::Block::new(
			&prev,
			vec![&coinbase_txn],
			&keychain,
			&key_id3,
			Difficulty::one(),
		).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	block.header.difficulty = difficulty.clone();

	match chain.set_sumtree_roots(&mut block, false) {
		Err(Error::ImmatureCoinbase) => (),
		_ => panic!("expected ImmatureCoinbase error here"),
	}

	pow::pow_size(
		&mut cuckoo_miner,
		&mut block.header,
		difficulty,
		global::sizeshift() as u32,
	).unwrap();

	// mine enough blocks to increase the height sufficiently for
	// coinbase to reach maturity and be spendable in the next block
	for _ in 0..3 {
		let prev = chain.head_header().unwrap();

		let keychain = Keychain::from_random_seed().unwrap();
		let pk = keychain.derive_key_id(1).unwrap();

		let mut block = core::core::Block::new(
			&prev,
			vec![],
			&keychain,
			&pk,
			Difficulty::one()
		).unwrap();
		block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

		let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
		block.header.difficulty = difficulty.clone();
		chain.set_sumtree_roots(&mut block, false).unwrap();

		pow::pow_size(
			&mut cuckoo_miner,
			&mut block.header,
			difficulty,
			global::sizeshift() as u32,
		).unwrap();

		chain.process_block(block, chain::MINE).unwrap();
	}

	let prev = chain.head_header().unwrap();

	let (coinbase_txn, _) = build::transaction(
		vec![
			build::coinbase_input(amount, block_hash, key_id1.clone()),
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
	block.header.difficulty = difficulty.clone();
	chain.set_sumtree_roots(&mut block, false).unwrap();

	pow::pow_size(
		&mut cuckoo_miner,
		&mut block.header,
		difficulty,
		global::sizeshift() as u32,
	).unwrap();

	let result = chain.process_block(block, chain::MINE);
	match result {
		Ok(_) => (),
		Err(Error::ImmatureCoinbase) => panic!("we should not get an ImmatureCoinbase here"),
		Err(_) => panic!("we did not expect an error here"),
	};
}
