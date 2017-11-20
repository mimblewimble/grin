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

use chain::Chain;
use chain::types::*;
use core::core::{Block, BlockHeader};
use core::core::hash::Hashed;
use core::core::target::Difficulty;
use core::consensus;
use core::global;
use core::global::ChainTypes;

use keychain::Keychain;

use pow::{cuckoo, types, MiningWorker};

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn setup(dir_name: &str) -> Chain {
	let _ = env_logger::init();
	clean_output_dir(dir_name);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block(None).unwrap();
	chain::Chain::init(
		dir_name.to_string(),
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
	).unwrap()
}

#[test]
fn mine_empty_chain() {
	let chain = setup(".grin");
	let keychain = Keychain::from_random_seed().unwrap();

	// mine and add a few blocks
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
	for n in 1..4 {
		let prev = chain.head_header().unwrap();
		let pk = keychain.derive_key_id(n as u32).unwrap();
		let mut b = core::core::Block::new(&prev, vec![], &keychain, &pk).unwrap();
		b.header.timestamp = prev.timestamp + time::Duration::seconds(60);

		let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
		b.header.difficulty = difficulty.clone();
		chain.set_sumtree_roots(&mut b).unwrap();

		pow::pow_size(
			&mut cuckoo_miner,
			&mut b.header,
			difficulty,
			global::sizeshift() as u32,
		).unwrap();

		let bhash = b.hash();
		chain.process_block(b, chain::NONE).unwrap();

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
		let header_by_output_commit = chain
			.get_block_header_by_output_commit(&output.commitment())
			.unwrap();
		assert_eq!(header_by_output_commit.hash(), bhash);
	}
}

#[test]
fn mine_forks() {
	let chain = setup(".grin2");

	// add a first block to not fork genesis
	let prev = chain.head_header().unwrap();
	let b = prepare_block(&prev, &chain, 2);
	chain.process_block(b, chain::SKIP_POW).unwrap();

	// mine and add a few blocks

	for n in 1..4 {
		// first block for one branch
		let prev = chain.head_header().unwrap();
		let b1 = prepare_block(&prev, &chain, 3 * n);

		// 2nd block with higher difficulty for other branch
		let b2 = prepare_block(&prev, &chain, 3 * n + 1);

		// process the first block to extend the chain
		let bhash = b1.hash();
		chain.process_block(b1, chain::SKIP_POW).unwrap();

		// checking our new head
		let head = chain.head().unwrap();
		assert_eq!(head.height, (n + 1) as u64);
		assert_eq!(head.last_block_h, bhash);
		assert_eq!(head.prev_block_h, prev.hash());

		// process the 2nd block to build a fork with more work
		let bhash = b2.hash();
		chain.process_block(b2, chain::SKIP_POW).unwrap();

		// checking head switch
		let head = chain.head().unwrap();
		assert_eq!(head.height, (n + 1) as u64);
		assert_eq!(head.last_block_h, bhash);
		assert_eq!(head.prev_block_h, prev.hash());
	}
}

#[test]
fn mine_losing_fork() {
	let chain = setup(".grin3");

	// add a first block we'll be forking from
	let prev = chain.head_header().unwrap();
	let b1 = prepare_block(&prev, &chain, 2);
	let b1head = b1.header.clone();
	chain.process_block(b1, chain::SKIP_POW).unwrap();

	// prepare the 2 successor, sibling blocks, one with lower diff
	let b2 = prepare_block(&b1head, &chain, 4);
	let b2head = b2.header.clone();
	let bfork = prepare_block(&b1head, &chain, 3);

	// add higher difficulty first, prepare its successor, then fork
 // with lower diff
	chain.process_block(b2, chain::SKIP_POW).unwrap();
	assert_eq!(chain.head_header().unwrap().hash(), b2head.hash());
	let b3 = prepare_block(&b2head, &chain, 5);
	chain.process_block(bfork, chain::SKIP_POW).unwrap();

	// adding the successor
	let b3head = b3.header.clone();
	chain.process_block(b3, chain::SKIP_POW).unwrap();
	assert_eq!(chain.head_header().unwrap().hash(), b3head.hash());
}

#[test]
fn longer_fork() {
	// to make it easier to compute the sumtree roots in the test, we
 // prepare 2 chains, the 2nd will be have the forked blocks we can
 // then send back on the 1st
	let chain = setup(".grin4");
	let chain_fork = setup(".grin5");

	// add blocks to both chains, 20 on the main one, only the first 5
 // for the forked chain
	let mut prev = chain.head_header().unwrap();
	for n in 0..10 {
		let b = prepare_block(&prev, &chain, n + 2);
		let bh = b.header.clone();

		if n < 5 {
			let b_fork = b.clone();
			chain_fork.process_block(b_fork, chain::SKIP_POW).unwrap();
		}

		chain.process_block(b, chain::SKIP_POW).unwrap();
		prev = bh;
	}

	// check both chains are in the expected state
	let head = chain.head_header().unwrap();
	assert_eq!(head.height, 10);
	assert_eq!(head.hash(), prev.hash());
	let head_fork = chain_fork.head_header().unwrap();
	assert_eq!(head_fork.height, 5);

	let mut prev_fork = head_fork.clone();
	for n in 0..7 {
		let b_fork = prepare_block(&prev_fork, &chain_fork, n + 7);
		let bh_fork = b_fork.header.clone();

		let b = b_fork.clone();
		chain.process_block(b, chain::SKIP_POW).unwrap();

		chain_fork.process_block(b_fork, chain::SKIP_POW).unwrap();
		prev_fork = bh_fork;
	}
}

fn prepare_block(prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = prepare_block_nosum(prev, diff);
	chain.set_sumtree_roots(&mut b).unwrap();
	b
}

fn prepare_block_nosum(prev: &BlockHeader, diff: u64) -> Block {
	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();

	let mut b = core::core::Block::new(prev, vec![], &keychain, &key_id).unwrap();
	b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
	b.header.total_difficulty = Difficulty::from_num(diff);
	b
}
