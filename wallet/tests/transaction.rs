// Copyright 2018 The Grin Developers
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

//! tests for transactions building within libwallet
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate rand;
extern crate time;

use std::fs;
use std::sync::Arc;

use chain::Chain;
use chain::types::*;
use core::core::{Block, BlockHeader, OutputFeatures, OutputIdentifier, Output, Transaction, TxKernel};
use core::core::hash::Hashed;
use core::core::target::Difficulty;
use core::consensus;
use core::global;
use core::global::ChainTypes;
use wallet::libwallet::{self, build};

use util::LOGGER;

use keychain::Keychain;

use core::pow;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn setup(dir_name: &str) -> Chain {
	let mut log_config = util::LoggingConfig::default();
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

fn add_block_with_reward(chain: &Chain, reward: (Output, TxKernel)) {
	let prev = chain.head_header().unwrap();
	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	let mut b = core::core::Block::new(&prev, vec![], difficulty.clone(), reward).unwrap();
	b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
	chain.set_txhashset_roots(&mut b, false).unwrap();
	pow::pow_size(
		&mut b.header,
		difficulty,
		global::proofsize(),
		global::sizeshift(),
	).unwrap();
	chain.process_block(b, chain::Options::MINE).unwrap();
	chain.validate(false).unwrap();
}

/// adds a reward output to a wallet, includes that reward in a block, mines the block
/// and adds it to the chain. Helpful for building up precise wallet balances for testing.
/*fn award_block_to_wallet(chain: &Chain, config: &WalletConfig, keychain: &Keychain) {
	let reward = libwallet::reward::output(&keychain, &pk, 0, prev.height).unwrap();
	/*let prev = chain.head_header().unwrap();
	let fees = BlockFees {
		fees: 0,
		key_id: 0,
		height: prev.height + 1,
	};
	wallet::receiver::receive_coinbase(config, keychain, BlockFees::new */
}*/

/// Build a transaction between 2 parties
#[test]
fn build_transaction() {
	let chain = setup(".build_transaction");
	let keychain = Keychain::from_random_seed().unwrap();
	let pk = keychain.derive_key_id(1).unwrap();
	let prev = chain.head_header().unwrap();
	let reward = libwallet::reward::output(&keychain, &pk, 0, prev.height).unwrap();
	add_block_with_reward(&chain, reward);
}
