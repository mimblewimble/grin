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
extern crate grin_util as util;
extern crate rand;
extern crate time;

use std::fs;
use std::sync::Arc;

use chain::Chain;
use chain::types::*;
use core::core::{Block, BlockHeader, Transaction};
use core::core::target::Difficulty;
use core::{consensus, genesis};
use core::global;
use core::global::ChainTypes;

use keychain::Keychain;

use core::pow;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn setup(dir_name: &str) -> Chain {
	util::init_test_logger();
	clean_output_dir(dir_name);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block().unwrap();
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

		for n in 1..4 {
			let prev = chain.head_header().unwrap();
			let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
			let pk = keychain.derive_key_id(n as u32).unwrap();
			let mut b =
				core::core::Block::new(&prev, vec![], &keychain, &pk, difficulty.clone()).unwrap();
			b.header.timestamp = prev.timestamp + time::Duration::seconds(60);

			chain.set_txhashset_roots(&mut b, false).unwrap();

			pow::pow_size(
				&mut b.header,
				difficulty,
				global::proofsize(),
				global::sizeshift(),
			).unwrap();

			let bhash = b.hash();
			chain
				.process_block(b.clone(), chain::Options::MINE)
				.unwrap();

			let head = Tip::from_block(&b.header);

			// Check we have block markers for the last block and the block previous
			let cur_pmmr_md = chain
				.get_block_marker(&head.last_block_h)
				.expect("block marker does not exist");
			chain
				.get_block_marker(&head.prev_block_h)
				.expect("prev block marker does not exist");

			println!("Cur_pmmr_md: {:?}", cur_pmmr_md);
			chain.validate(false).unwrap();
		}
	}
	// Now reload the chain, should have valid indices
	{
		let chain = reload_chain(chain_dir);
		chain.validate(false).unwrap();
	}
}

fn prepare_block(kc: &Keychain, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots(&mut b, false).unwrap();
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
	chain.set_txhashset_roots(&mut b, false).unwrap();
	b
}

fn prepare_fork_block(kc: &Keychain, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots(&mut b, true).unwrap();
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
	chain.set_txhashset_roots(&mut b, true).unwrap();
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
