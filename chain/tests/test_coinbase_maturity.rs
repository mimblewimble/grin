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
use rand::os::OsRng;

use chain::types::*;
use core::core::build;
use core::core::transaction;
use core::consensus;
use core::global;
use core::global::MiningParameterMode;

use pow::{types, cuckoo, MiningWorker};

fn clean_output_dir(dir_name:&str){
    let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_coinbase_maturity() {
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

	let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

	let mut miner_config = types::MinerConfig {
		enable_mining: true,
		burn_reward: true,
		..Default::default()
	};
	miner_config.cuckoo_miner_plugin_dir = Some(String::from("../target/debug/deps"));

	let mut cuckoo_miner = cuckoo::Miner::new(consensus::EASINESS, global::sizeshift() as u32, global::proofsize());

	let prev = chain.head_header().unwrap();
    let reward_key = secp::key::SecretKey::new(&secp, &mut rng);
	let mut block = core::core::Block::new(&prev, vec![], reward_key).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	block.header.difficulty = difficulty.clone();

	pow::pow_size(
		&mut cuckoo_miner,
		&mut block.header,
		difficulty,
		global::sizeshift() as u32,
	).unwrap();

    assert_eq!(block.outputs.len(), 1);
    assert!(block.outputs[0].features.contains(transaction::COINBASE_OUTPUT));

	chain.process_block(block, chain::EASY_POW).unwrap();

    let prev = chain.head_header().unwrap();

    let amount = consensus::REWARD;
    let (coinbase_txn, _) = build::transaction(vec![
        build::input(amount, reward_key),
        build::output_rand(amount-1),
        build::with_fee(1)]
    ).unwrap();

    let reward_key = secp::key::SecretKey::new(&secp, &mut rng);
	let mut block = core::core::Block::new(&prev, vec![&coinbase_txn], reward_key).unwrap();

	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	block.header.difficulty = difficulty.clone();

	pow::pow_size(
		&mut cuckoo_miner,
		&mut block.header,
		difficulty,
		global::sizeshift() as u32,
	).unwrap();

	let result = chain.process_block(block, chain::EASY_POW);
    match result {
        Err(Error::ImmatureCoinbase) => (),
        _ => panic!("expected ImmatureCoinbase error here"),
    };

    // mine 10 blocks so we increase the height sufficiently
    // coinbase will mature and be spendable in the block after these
    for _ in 0..10 {
        let prev = chain.head_header().unwrap();

        let reward_key = secp::key::SecretKey::new(&secp, &mut rng);
        let mut block = core::core::Block::new(&prev, vec![], reward_key).unwrap();
        block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

        let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
        block.header.difficulty = difficulty.clone();

        pow::pow_size(
            &mut cuckoo_miner,
            &mut block.header,
            difficulty,
            global::sizeshift() as u32,
        ).unwrap();

        chain.process_block(block, chain::EASY_POW).unwrap();
    };

    let prev = chain.head_header().unwrap();

    let reward_key = secp::key::SecretKey::new(&secp, &mut rng);
    let mut block = core::core::Block::new(&prev, vec![&coinbase_txn], reward_key).unwrap();

    block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

    let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
    block.header.difficulty = difficulty.clone();

    pow::pow_size(
        &mut cuckoo_miner,
        &mut block.header,
        difficulty,
        global::sizeshift() as u32,
    ).unwrap();

    let result = chain.process_block(block, chain::EASY_POW);
    match result {
        Ok(_) => (),
        Err(Error::ImmatureCoinbase) => panic!("we should not get an ImmatureCoinbase here"),
        Err(_) => panic!("we did not expect an error here"),
    };
}
