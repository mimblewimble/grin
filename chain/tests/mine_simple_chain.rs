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

extern crate grin_core as core;
extern crate grin_chain as chain;
extern crate env_logger;
extern crate time;
extern crate rand;
extern crate secp256k1zkp as secp;
extern crate grin_pow as pow;

use std::fs;
use std::sync::Arc;
use std::thread;
use rand::os::OsRng;

use chain::types::*;
use core::core::hash::Hashed;
use core::core::target::Difficulty;
use core::consensus;
use core::global;
use core::global::MiningParameterMode;

use pow::{types, cuckoo, MiningWorker};

fn clean_output_dir(dir_name:&str){
    let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn mine_empty_chain() {
    let _ = env_logger::init();
	clean_output_dir(".grin");
    global::set_mining_mode(MiningParameterMode::AutomatedTesting);

	let mut rng = OsRng::new().unwrap();
	let mut genesis_block = None;
	if !chain::Chain::chain_exists(".grin".to_string()){
		genesis_block=pow::mine_genesis_block(None);
	}
	let chain = chain::Chain::init(".grin".to_string(), Arc::new(NoopAdapter {}),
									genesis_block, pow::verify_size).unwrap();

	// mine and add a few blocks
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let reward_key = secp::key::SecretKey::new(&secp, &mut rng);

	let mut miner_config = types::MinerConfig {
		enable_mining: true,
		burn_reward: true,
		..Default::default()
	};
	miner_config.cuckoo_miner_plugin_dir = Some(String::from("../target/debug/deps"));

	let mut cuckoo_miner = cuckoo::Miner::new(consensus::EASINESS, global::sizeshift() as u32, global::proofsize());
	for n in 1..4 {
		let prev = chain.head_header().unwrap();
		let mut b = core::core::Block::new(&prev, vec![], reward_key).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);

		let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
		b.header.difficulty = difficulty.clone();

		pow::pow_size(
			&mut cuckoo_miner,
			&mut b.header,
			difficulty,
			global::sizeshift() as u32,
		).unwrap();

		let bhash = b.hash();
		chain.process_block(b, chain::EASY_POW).unwrap();

		// checking our new head
		let head = chain.head().unwrap();
		assert_eq!(head.height, n);
		assert_eq!(head.last_block_h, bhash);

        // now check the block_header of the head
        let header = chain.head_header().unwrap();
        assert_eq!(header.height, n);
        assert_eq!(header.hash(), bhash);

        // now check the block itself
        let block = chain.get_block(&header.hash()).unwrap();
        assert_eq!(block.header.height, n);
        assert_eq!(block.hash(), bhash);
        assert_eq!(block.outputs.len(), 1);

        // now check the block height index
        let header_by_height = chain.get_header_by_height(n).unwrap();
        assert_eq!(header_by_height.hash(), bhash);

        // now check the header output index
        let output = block.outputs[0];
        let header_by_output_commit = chain.get_block_header_by_output_commit(&output.commitment()).unwrap();
        assert_eq!(header_by_output_commit.hash(), bhash);
	}
}

#[test]
fn mine_forks() {
    let _ = env_logger::init();
	clean_output_dir(".grin2");

	let mut rng = OsRng::new().unwrap();

	let mut genesis_block = None;
	if !chain::Chain::chain_exists(".grin2".to_string()){
		genesis_block=pow::mine_genesis_block(None);
	}
	let chain = chain::Chain::init(".grin2".to_string(), Arc::new(NoopAdapter {}),
									genesis_block, pow::verify_size).unwrap();

	// mine and add a few blocks
	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
	let reward_key = secp::key::SecretKey::new(&secp, &mut rng);

	for n in 1..4 {
		let prev = chain.head_header().unwrap();
		let mut b = core::core::Block::new(&prev, vec![], reward_key).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
		b.header.total_difficulty = Difficulty::from_num(2 * n);
		let bhash = b.hash();
		chain.process_block(b, chain::SKIP_POW).unwrap();

		// checking our new head
		thread::sleep(::std::time::Duration::from_millis(50));
		let head = chain.head().unwrap();
		assert_eq!(head.height, n as u64);
		assert_eq!(head.last_block_h, bhash);
		assert_eq!(head.prev_block_h, prev.hash());

		// build another block with higher difficulty
		let mut b = core::core::Block::new(&prev, vec![], reward_key).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
		b.header.total_difficulty = Difficulty::from_num(2 * n + 1);
		let bhash = b.hash();
		chain.process_block(b, chain::SKIP_POW).unwrap();

		// checking head switch
		thread::sleep(::std::time::Duration::from_millis(50));
		let head = chain.head().unwrap();
		assert_eq!(head.height, n as u64);
		assert_eq!(head.last_block_h, bhash);
		assert_eq!(head.prev_block_h, prev.hash());
	}
}
