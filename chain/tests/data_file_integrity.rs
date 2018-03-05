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
extern crate grin_util as util;
extern crate rand;
extern crate time;

use std::fs;
use std::sync::Arc;

use chain::Chain;
use chain::types::*;
use core::core::{Block, BlockHeader, Transaction};
use core::core::hash::Hashed;
use core::core::target::Difficulty;
use core::{consensus, genesis};
use core::global;
use core::global::ChainTypes;

use keychain::Keychain;

use pow::{cuckoo, types, MiningWorker};

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn setup(dir_name: &str) -> Chain {
	util::init_test_logger();
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

fn reload_chain(dir_name: &str) -> Chain {
	chain::Chain::init(
		dir_name.to_string(),
		Arc::new(NoopAdapter {}),
		genesis::genesis_dev(),
		pow::verify_size,
	).unwrap()
}

#[test]
fn data_files() {
	let chain_dir = ".grin_df";
	//new block so chain references should be freed
	{
		let chain = setup(chain_dir);
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
			let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
			let pk = keychain.derive_key_id(n as u32).unwrap();
			let mut b =
				core::core::Block::new(&prev, vec![], &keychain, &pk, difficulty.clone()).unwrap();
			b.header.timestamp = prev.timestamp + time::Duration::seconds(60);

			b.header.difficulty = difficulty.clone(); // TODO: overwrite here? really?
			chain.set_sumtree_roots(&mut b, false).unwrap();

			pow::pow_size(
				&mut cuckoo_miner,
				&mut b.header,
				difficulty,
				global::sizeshift() as u32,
			).unwrap();

			let prev_bhash = b.header.previous;
			let bhash = b.hash();
			chain
				.process_block(b.clone(), chain::Options::MINE)
				.unwrap();

			let head = Tip::from_block(&b.header);

			// Check we have indexes for the last block and the block previous
			let cur_pmmr_md = chain
				.get_block_pmmr_file_metadata(&head.last_block_h)
				.expect("block pmmr file data doesn't exist");
			let pref_pmmr_md = chain
				.get_block_pmmr_file_metadata(&head.prev_block_h)
				.expect("previous block pmmr file data doesn't exist");

			println!("Cur_pmmr_md: {:?}", cur_pmmr_md);
			chain.validate().unwrap();
		}
	}
	// Now reload the chain, should have valid indices
	{
		let chain = reload_chain(chain_dir);
		chain.validate().unwrap();
	}
}

fn prepare_block(kc: &Keychain, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_sumtree_roots(&mut b, false).unwrap();
	b
}

fn prepare_block_tx(
	kc: &Keychain,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block {
	let mut b = prepare_block_nosum(kc, prev, diff, txs);
	chain.set_sumtree_roots(&mut b, false).unwrap();
	b
}

fn prepare_fork_block(kc: &Keychain, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_sumtree_roots(&mut b, true).unwrap();
	b
}

fn prepare_fork_block_tx(
	kc: &Keychain,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block {
	let mut b = prepare_block_nosum(kc, prev, diff, txs);
	chain.set_sumtree_roots(&mut b, true).unwrap();
	b
}

fn prepare_block_nosum(
	kc: &Keychain,
	prev: &BlockHeader,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block {
	let key_id = kc.derive_key_id(diff as u32).unwrap();

	let mut b = match core::core::Block::new(prev, txs, kc, &key_id, Difficulty::from_num(diff)) {
		Err(e) => panic!("{:?}", e),
		Ok(b) => b,
	};
	b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
	b.header.total_difficulty = Difficulty::from_num(diff);
	b
}
