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

extern crate chrono;
extern crate env_logger;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate rand;

use chrono::Duration;
use std::fs;
use std::sync::Arc;

use chain::types::{NoopAdapter, Tip};
use chain::Chain;
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader, Transaction};
use core::global::{self, ChainTypes};
use core::pow;
use core::{consensus, genesis};
use keychain::{ExtKeychain, Keychain};
use wallet::libtx;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn setup(dir_name: &str) -> Chain {
	util::init_test_logger();
	clean_output_dir(dir_name);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block().unwrap();
	let db_env = Arc::new(store::new_env(dir_name.to_string()));
	chain::Chain::init(
		dir_name.to_string(),
		db_env,
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
	).unwrap()
}

fn reload_chain(dir_name: &str) -> Chain {
	let db_env = Arc::new(store::new_env(dir_name.to_string()));
	chain::Chain::init(
		dir_name.to_string(),
		db_env,
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
		let keychain = ExtKeychain::from_random_seed().unwrap();

		for n in 1..4 {
			let prev = chain.head_header().unwrap();
			let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
			let pk = keychain.derive_key_id(n as u32).unwrap();
			let reward = libtx::reward::output(&keychain, &pk, 0, prev.height).unwrap();
			let mut b = core::core::Block::new(&prev, vec![], difficulty.clone(), reward).unwrap();
			b.header.timestamp = prev.timestamp + Duration::seconds(60);

			chain.set_txhashset_roots(&mut b, false).unwrap();

			pow::pow_size(
				&mut b.header,
				difficulty,
				global::proofsize(),
				global::min_sizeshift(),
			).unwrap();

			let _bhash = b.hash();
			chain
				.process_block(b.clone(), chain::Options::MINE)
				.unwrap();

			chain.validate(false).unwrap();
		}
	}
	// Now reload the chain, should have valid indices
	{
		let chain = reload_chain(chain_dir);
		chain.validate(false).unwrap();
	}
}

fn _prepare_block(kc: &ExtKeychain, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = _prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots(&mut b, false).unwrap();
	b
}

fn _prepare_block_tx(
	kc: &ExtKeychain,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block {
	let mut b = _prepare_block_nosum(kc, prev, diff, txs);
	chain.set_txhashset_roots(&mut b, false).unwrap();
	b
}

fn _prepare_fork_block(kc: &ExtKeychain, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block {
	let mut b = _prepare_block_nosum(kc, prev, diff, vec![]);
	chain.set_txhashset_roots(&mut b, true).unwrap();
	b
}

fn _prepare_fork_block_tx(
	kc: &ExtKeychain,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block {
	let mut b = _prepare_block_nosum(kc, prev, diff, txs);
	chain.set_txhashset_roots(&mut b, true).unwrap();
	b
}

fn _prepare_block_nosum(
	kc: &ExtKeychain,
	prev: &BlockHeader,
	diff: u64,
	txs: Vec<&Transaction>,
) -> Block {
	let key_id = kc.derive_key_id(diff as u32).unwrap();

	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward = libtx::reward::output(kc, &key_id, fees, prev.height).unwrap();
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
	b.header.total_difficulty = Difficulty::from_num(diff);
	b
}
