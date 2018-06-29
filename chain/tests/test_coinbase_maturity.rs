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
extern crate grin_store as store;
extern crate grin_wallet as wallet;
extern crate rand;
extern crate time;

use std::fs;
use std::sync::Arc;

use chain::types::{Error, NoopAdapter};
use core::core::target::Difficulty;
use core::core::{transaction, OutputIdentifier};
use core::global::{self, ChainTypes};
use core::{consensus, pow};
use keychain::{ExtKeychain, Keychain};
use wallet::libtx::{self, build};

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_coinbase_maturity() {
	let _ = env_logger::init();
	clean_output_dir(".grin");
	global::set_mining_mode(ChainTypes::AutomatedTesting);

	let genesis_block = pow::mine_genesis_block().unwrap();

	let db_env = Arc::new(store::new_env(".grin".to_string()));
	let chain = chain::Chain::init(
		".grin".to_string(),
		db_env,
		Arc::new(NoopAdapter {}),
		genesis_block,
		pow::verify_size,
	).unwrap();

	let prev = chain.head_header().unwrap();

	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();
	let key_id4 = keychain.derive_key_id(4).unwrap();

	let reward = libtx::reward::output(&keychain, &key_id1, 0, prev.height).unwrap();
	let mut block = core::core::Block::new(&prev, vec![], Difficulty::one(), reward).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

	chain.set_txhashset_roots(&mut block, false).unwrap();

	pow::pow_size(
		&mut block.header,
		difficulty,
		global::proofsize(),
		global::min_sizeshift(),
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
			build::coinbase_input(amount, key_id1.clone()),
			build::output(amount - 2, key_id2.clone()),
			build::with_fee(2),
		],
		&keychain,
	).unwrap();

	let txs = vec![coinbase_txn.clone()];
	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward = libtx::reward::output(&keychain, &key_id3, fees, prev.height).unwrap();
	let mut block = core::core::Block::new(&prev, txs, Difficulty::one(), reward).unwrap();
	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

	chain.set_txhashset_roots(&mut block, false).unwrap();

	// Confirm the tx attempting to spend the coinbase output
	// is not valid at the current block height given the current chain state.
	match chain.verify_coinbase_maturity(&coinbase_txn) {
		Err(Error::ImmatureCoinbase) => {}
		_ => panic!("Expected transaction error with immature coinbase."),
	}

	pow::pow_size(
		&mut block.header,
		difficulty,
		global::proofsize(),
		global::min_sizeshift(),
	).unwrap();

	// mine enough blocks to increase the height sufficiently for
	// coinbase to reach maturity and be spendable in the next block
	for _ in 0..3 {
		let prev = chain.head_header().unwrap();

		let keychain = ExtKeychain::from_random_seed().unwrap();
		let pk = keychain.derive_key_id(1).unwrap();

		let reward = libtx::reward::output(&keychain, &pk, 0, prev.height).unwrap();
		let mut block = core::core::Block::new(&prev, vec![], Difficulty::one(), reward).unwrap();
		block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

		let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

		chain.set_txhashset_roots(&mut block, false).unwrap();

		pow::pow_size(
			&mut block.header,
			difficulty,
			global::proofsize(),
			global::min_sizeshift(),
		).unwrap();

		chain.process_block(block, chain::Options::MINE).unwrap();
	}

	let prev = chain.head_header().unwrap();

	// Confirm the tx spending the coinbase output is now valid.
	// The coinbase output has matured sufficiently based on current chain state.
	chain.verify_coinbase_maturity(&coinbase_txn).unwrap();

	let txs = vec![coinbase_txn];
	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward = libtx::reward::output(&keychain, &key_id4, fees, prev.height).unwrap();
	let mut block = core::core::Block::new(&prev, txs, Difficulty::one(), reward).unwrap();

	block.header.timestamp = prev.timestamp + time::Duration::seconds(60);

	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();

	chain.set_txhashset_roots(&mut block, false).unwrap();

	pow::pow_size(
		&mut block.header,
		difficulty,
		global::proofsize(),
		global::min_sizeshift(),
	).unwrap();

	let result = chain.process_block(block, chain::Options::MINE);
	match result {
		Ok(_) => (),
		Err(_) => panic!("we did not expect an error here"),
	};
}
