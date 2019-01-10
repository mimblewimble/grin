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

use self::chain::types::NoopAdapter;
use self::chain::Chain;
use self::core::core::hash::Hashed;
use self::core::core::verifier_cache::LruVerifierCache;
use self::core::core::{Block, BlockHeader, OutputIdentifier, Transaction};
use self::core::genesis;
use self::core::global::ChainTypes;
use self::core::libtx::{self, build, reward};
use self::core::pow::Difficulty;
use self::core::{consensus, global, pow};
use self::keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use self::util::{Mutex, RwLock, StopState};
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_store as store;
use grin_util as util;
use std::fs;
use std::sync::Arc;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn setup(dir_name: &str, genesis: Block) -> Chain {
	util::init_test_logger();
	clean_output_dir(dir_name);
	let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));
	let db_env = Arc::new(store::new_env(dir_name.to_string()));
	chain::Chain::init(
		dir_name.to_string(),
		db_env,
		Arc::new(NoopAdapter {}),
		genesis,
		pow::verify_size,
		verifier_cache,
		false,
		Arc::new(Mutex::new(StopState::new())),
	)
	.unwrap()
}

#[test]
fn mine_empty_chain() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	mine_some_on_top(".grin", pow::mine_genesis_block().unwrap(), &keychain);
}

#[test]
fn mine_genesis_reward_chain() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	// add coinbase data from the dev genesis block
	let mut genesis = genesis::genesis_dev();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let key_id = keychain::ExtKeychain::derive_key_id(0, 1, 0, 0, 0);
	let reward = reward::output(&keychain, &key_id, 0).unwrap();
	genesis = genesis.with_reward(reward.0, reward.1);

	{
		// setup a tmp chain to hande tx hashsets
		let tmp_chain = setup(".grin.tmp", pow::mine_genesis_block().unwrap());
		tmp_chain.set_txhashset_roots(&mut genesis).unwrap();
		genesis.header.output_mmr_size = 1;
		genesis.header.kernel_mmr_size = 1;
	}

	// get a valid PoW
	pow::pow_size(
		&mut genesis.header,
		Difficulty::unit(),
		global::proofsize(),
		global::min_edge_bits(),
	)
	.unwrap();

	mine_some_on_top(".grin.genesis", genesis, &keychain);
}

fn mine_some_on_top<K>(dir: &str, genesis: Block, keychain: &K)
where
	K: Keychain,
{
	let chain = setup(dir, genesis);

	for n in 1..4 {
		let prev = chain.head_header().unwrap();
		let next_header_info = consensus::next_difficulty(1, chain.difficulty_iter());
		let pk = ExtKeychainPath::new(1, n as u32, 0, 0, 0).to_identifier();
		let reward = libtx::reward::output(keychain, &pk, 0).unwrap();
		let mut b =
			core::core::Block::new(&prev, vec![], next_header_info.clone().difficulty, reward)
				.unwrap();
		b.header.timestamp = prev.timestamp + Duration::seconds(60);
		b.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		chain.set_txhashset_roots(&mut b).unwrap();

		let edge_bits = if n == 2 {
			global::min_edge_bits() + 1
		} else {
			global::min_edge_bits()
		};
		b.header.pow.proof.edge_bits = edge_bits;
		pow::pow_size(
			&mut b.header,
			next_header_info.difficulty,
			global::proofsize(),
			edge_bits,
		)
		.unwrap();
		b.header.pow.proof.edge_bits = edge_bits;

		let bhash = b.hash();
		chain.process_block(b, chain::Options::MINE).unwrap();

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
		assert_eq!(block.outputs().len(), 1);

		// now check the block height index
		let header_by_height = chain.get_header_by_height(n).unwrap();
		assert_eq!(header_by_height.hash(), bhash);

		chain.validate(false).unwrap();
	}
}

#[test]
fn mine_forks() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let chain = setup(".grin2", pow::mine_genesis_block().unwrap());
	let kc = ExtKeychain::from_random_seed(false).unwrap();

	// add a first block to not fork genesis
	let prev = chain.head_header().unwrap();
	let b = prepare_block(&kc, &prev, &chain, 2);
	chain.process_block(b, chain::Options::SKIP_POW).unwrap();

	// mine and add a few blocks

	for n in 1..4 {
		// first block for one branch
		let prev = chain.head_header().unwrap();
		let b1 = prepare_block(&kc, &prev, &chain, 3 * n);

		// 2nd block with higher difficulty for other branch
		let b2 = prepare_block(&kc, &prev, &chain, 3 * n + 1);

		// process the first block to extend the chain
		let bhash = b1.hash();
		chain.process_block(b1, chain::Options::SKIP_POW).unwrap();

		// checking our new head
		let head = chain.head().unwrap();
		assert_eq!(head.height, (n + 1) as u64);
		assert_eq!(head.last_block_h, bhash);
		assert_eq!(head.prev_block_h, prev.hash());

		// process the 2nd block to build a fork with more work
		let bhash = b2.hash();
		chain.process_block(b2, chain::Options::SKIP_POW).unwrap();

		// checking head switch
		let head = chain.head().unwrap();
		assert_eq!(head.height, (n + 1) as u64);
		assert_eq!(head.last_block_h, bhash);
		assert_eq!(head.prev_block_h, prev.hash());
	}
}

#[test]
fn mine_losing_fork() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	let chain = setup(".grin3", pow::mine_genesis_block().unwrap());

	// add a first block we'll be forking from
	let prev = chain.head_header().unwrap();
	let b1 = prepare_block(&kc, &prev, &chain, 2);
	let b1head = b1.header.clone();
	chain.process_block(b1, chain::Options::SKIP_POW).unwrap();

	// prepare the 2 successor, sibling blocks, one with lower diff
	let b2 = prepare_block(&kc, &b1head, &chain, 4);
	let b2head = b2.header.clone();
	let bfork = prepare_block(&kc, &b1head, &chain, 3);

	// add higher difficulty first, prepare its successor, then fork
	// with lower diff
	chain.process_block(b2, chain::Options::SKIP_POW).unwrap();
	assert_eq!(chain.head_header().unwrap().hash(), b2head.hash());
	let b3 = prepare_block(&kc, &b2head, &chain, 5);
	chain
		.process_block(bfork, chain::Options::SKIP_POW)
		.unwrap();

	// adding the successor
	let b3head = b3.header.clone();
	chain.process_block(b3, chain::Options::SKIP_POW).unwrap();
	assert_eq!(chain.head_header().unwrap().hash(), b3head.hash());
}

#[test]
fn longer_fork() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	// to make it easier to compute the txhashset roots in the test, we
	// prepare 2 chains, the 2nd will be have the forked blocks we can
	// then send back on the 1st
	let genesis = pow::mine_genesis_block().unwrap();
	let chain = setup(".grin4", genesis.clone());

	// add blocks to both chains, 20 on the main one, only the first 5
	// for the forked chain
	let mut prev = chain.head_header().unwrap();
	for n in 0..10 {
		let b = prepare_block(&kc, &prev, &chain, 2 * n + 2);
		prev = b.header.clone();
		chain.process_block(b, chain::Options::SKIP_POW).unwrap();
	}

	let forked_block = chain.get_header_by_height(5).unwrap();

	let head = chain.head_header().unwrap();
	assert_eq!(head.height, 10);
	assert_eq!(head.hash(), prev.hash());

	let mut prev = forked_block;
	for n in 0..7 {
		let b = prepare_fork_block(&kc, &prev, &chain, 2 * n + 11);
		prev = b.header.clone();
		chain.process_block(b, chain::Options::SKIP_POW).unwrap();
	}

	let new_head = prev;

	// After all this the chain should have switched to the fork.
	let head = chain.head_header().unwrap();
	assert_eq!(head.height, 12);
	assert_eq!(head.hash(), new_head.hash());
}

#[test]
fn spend_in_fork_and_compact() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	util::init_test_logger();
	let chain = setup(".grin6", pow::mine_genesis_block().unwrap());
	let prev = chain.head_header().unwrap();
	let kc = ExtKeychain::from_random_seed(false).unwrap();

	let mut fork_head = prev;

	// mine the first block and keep track of the block_hash
	// so we can spend the coinbase later
	let b = prepare_block(&kc, &fork_head, &chain, 2);
	let out_id = OutputIdentifier::from_output(&b.outputs()[0]);
	assert!(out_id.features.is_coinbase());
	fork_head = b.header.clone();
	chain
		.process_block(b.clone(), chain::Options::SKIP_POW)
		.unwrap();

	// now mine three further blocks
	for n in 3..6 {
		let b = prepare_block(&kc, &fork_head, &chain, n);
		fork_head = b.header.clone();
		chain.process_block(b, chain::Options::SKIP_POW).unwrap();
	}

	// Check the height of the "fork block".
	assert_eq!(fork_head.height, 4);
	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
	let key_id30 = ExtKeychainPath::new(1, 30, 0, 0, 0).to_identifier();
	let key_id31 = ExtKeychainPath::new(1, 31, 0, 0, 0).to_identifier();

	let tx1 = build::transaction(
		vec![
			build::coinbase_input(consensus::REWARD, key_id2.clone()),
			build::output(consensus::REWARD - 20000, key_id30.clone()),
			build::with_fee(20000),
		],
		&kc,
	)
	.unwrap();

	let next = prepare_block_tx(&kc, &fork_head, &chain, 7, vec![&tx1]);
	let prev_main = next.header.clone();
	chain
		.process_block(next.clone(), chain::Options::SKIP_POW)
		.unwrap();
	chain.validate(false).unwrap();

	let tx2 = build::transaction(
		vec![
			build::input(consensus::REWARD - 20000, key_id30.clone()),
			build::output(consensus::REWARD - 40000, key_id31.clone()),
			build::with_fee(20000),
		],
		&kc,
	)
	.unwrap();

	let next = prepare_block_tx(&kc, &prev_main, &chain, 9, vec![&tx2]);
	let prev_main = next.header.clone();
	chain.process_block(next, chain::Options::SKIP_POW).unwrap();

	// Full chain validation for completeness.
	chain.validate(false).unwrap();

	// mine 2 forked blocks from the first
	let fork = prepare_fork_block_tx(&kc, &fork_head, &chain, 6, vec![&tx1]);
	let prev_fork = fork.header.clone();
	chain.process_block(fork, chain::Options::SKIP_POW).unwrap();

	let fork_next = prepare_fork_block_tx(&kc, &prev_fork, &chain, 8, vec![&tx2]);
	let prev_fork = fork_next.header.clone();
	chain
		.process_block(fork_next, chain::Options::SKIP_POW)
		.unwrap();

	chain.validate(false).unwrap();

	// check state
	let head = chain.head_header().unwrap();
	assert_eq!(head.height, 6);
	assert_eq!(head.hash(), prev_main.hash());
	assert!(chain
		.is_unspent(&OutputIdentifier::from_output(&tx2.outputs()[0]))
		.is_ok());
	assert!(chain
		.is_unspent(&OutputIdentifier::from_output(&tx1.outputs()[0]))
		.is_err());

	// make the fork win
	let fork_next = prepare_fork_block(&kc, &prev_fork, &chain, 10);
	let prev_fork = fork_next.header.clone();
	chain
		.process_block(fork_next, chain::Options::SKIP_POW)
		.unwrap();
	chain.validate(false).unwrap();

	// check state
	let head = chain.head_header().unwrap();
	assert_eq!(head.height, 7);
	assert_eq!(head.hash(), prev_fork.hash());
	assert!(chain
		.is_unspent(&OutputIdentifier::from_output(&tx2.outputs()[0]))
		.is_ok());
	assert!(chain
		.is_unspent(&OutputIdentifier::from_output(&tx1.outputs()[0]))
		.is_err());

	// add 20 blocks to go past the test horizon
	let mut prev = prev_fork;
	for n in 0..20 {
		let next = prepare_block(&kc, &prev, &chain, 11 + n);
		prev = next.header.clone();
		chain.process_block(next, chain::Options::SKIP_POW).unwrap();
	}

	chain.validate(false).unwrap();
	if let Err(e) = chain.compact() {
		panic!("Error compacting chain: {:?}", e);
	}
	if let Err(e) = chain.validate(false) {
		panic!("Validation error after compacting chain: {:?}", e);
	}
}

/// Test ability to retrieve block headers for a given output
#[test]
fn output_header_mappings() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let chain = setup(
		".grin_header_for_output",
		pow::mine_genesis_block().unwrap(),
	);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let mut reward_outputs = vec![];

	for n in 1..15 {
		let prev = chain.head_header().unwrap();
		let next_header_info = consensus::next_difficulty(1, chain.difficulty_iter());
		let pk = ExtKeychainPath::new(1, n as u32, 0, 0, 0).to_identifier();
		let reward = libtx::reward::output(&keychain, &pk, 0).unwrap();
		reward_outputs.push(reward.0.clone());
		let mut b =
			core::core::Block::new(&prev, vec![], next_header_info.clone().difficulty, reward)
				.unwrap();
		b.header.timestamp = prev.timestamp + Duration::seconds(60);
		b.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		chain.set_txhashset_roots(&mut b).unwrap();

		let edge_bits = if n == 2 {
			global::min_edge_bits() + 1
		} else {
			global::min_edge_bits()
		};
		b.header.pow.proof.edge_bits = edge_bits;
		pow::pow_size(
			&mut b.header,
			next_header_info.difficulty,
			global::proofsize(),
			edge_bits,
		)
		.unwrap();
		b.header.pow.proof.edge_bits = edge_bits;

		chain.process_block(b, chain::Options::MINE).unwrap();

		let header_for_output = chain
			.get_header_for_output(&OutputIdentifier::from_output(&reward_outputs[n - 1]))
			.unwrap();
		assert_eq!(header_for_output.height, n as u64);

		chain.validate(false).unwrap();
	}

	// Check all output positions are as expected
	for n in 1..15 {
		let header_for_output = chain
			.get_header_for_output(&OutputIdentifier::from_output(&reward_outputs[n - 1]))
			.unwrap();
		assert_eq!(header_for_output.height, n as u64);
	}
}

fn prepare_block<K>(kc: &K, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

fn prepare_block_tx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, txs);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

fn prepare_fork_block<K>(kc: &K, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots_forked(&mut b, prev).unwrap();
	b
}

fn prepare_fork_block_tx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, txs);
	chain.set_txhashset_roots_forked(&mut b, prev).unwrap();
	b
}

fn prepare_block_nosum<K>(kc: &K, prev: &BlockHeader, diff: u64, txs: Vec<&Transaction>) -> Block
where
	K: Keychain,
{
	let proof_size = global::proofsize();
	let key_id = ExtKeychainPath::new(1, diff as u32, 0, 0, 0).to_identifier();

	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward = libtx::reward::output(kc, &key_id, fees).unwrap();
	let mut b = match core::core::Block::new(
		prev,
		txs.into_iter().cloned().collect(),
		Difficulty::from_num(diff),
		reward,
	) {
		Err(e) => panic!("{:?}", e),
		Ok(b) => b,
	};
	b.header.timestamp = prev.timestamp + Duration::seconds(60);
	b.header.pow.total_difficulty = prev.total_difficulty() + Difficulty::from_num(diff);
	b.header.pow.proof = pow::Proof::random(proof_size);
	b
}

#[test]
#[ignore]
fn actual_diff_iter_output() {
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block().unwrap();
	let db_env = Arc::new(store::new_env(".grin".to_string()));
	let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));
	let chain = chain::Chain::init(
		"../.grin".to_string(),
		db_env,
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
		verifier_cache,
		false,
		Arc::new(Mutex::new(StopState::new())),
	)
	.unwrap();
	let iter = chain.difficulty_iter();
	let mut last_time = 0;
	let mut first = true;
	for elem in iter.into_iter() {
		if first {
			last_time = elem.timestamp;
			first = false;
		}
		println!(
			"next_difficulty time: {}, diff: {}, duration: {} ",
			elem.timestamp,
			elem.difficulty.to_num(),
			last_time - elem.timestamp
		);
		last_time = elem.timestamp;
	}
}
