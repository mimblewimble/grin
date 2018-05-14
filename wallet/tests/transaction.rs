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
#[macro_use]
extern crate slog;
extern crate time;
extern crate uuid;

mod common;

use std::collections::hash_map::Entry;
use std::fs;
use std::sync::Arc;

use uuid::Uuid;

use chain::Chain;
use chain::types::*;
use core::core::{Block, BlockHeader, Output, OutputFeatures, OutputIdentifier, Transaction,
                 TxKernel};
use core::core::hash::Hashed;
use core::core::target::Difficulty;
use core::consensus;
use core::global;
use core::global::ChainTypes;
use wallet::libwallet::{self, aggsig, build, transaction};
use wallet::types::{BlockIdentifier, MerkleProofWrapper, WalletConfig, WalletData};
use wallet::BlockFees;
use wallet::checker;

use util::LOGGER;

use keychain::{Identifier, Keychain};

use core::pow;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str, chain_dir: &str) -> Chain {
	let log_config = util::LoggingConfig::default();
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
	let genesis_block = pow::mine_genesis_block().unwrap();
	let dir_name = format!("{}/{}", test_dir, chain_dir);
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
fn award_block_to_wallet(chain: &Chain, wallet: &(WalletConfig, Keychain)) {
	let prev = chain.head_header().unwrap();
	let fees = BlockFees {
		fees: 0,
		key_id: None,
		height: prev.height + 1,
	};
	let coinbase_tx = wallet::receiver::receive_coinbase(&wallet.0, &wallet.1, &fees);
	let (coinbase_tx, fees) = match coinbase_tx {
		Ok(t) => ((t.0, t.1), t.2),
		Err(e) => {
			panic!("Unable to create block reward: {:?}", e);
		}
	};
	add_block_with_reward(chain, coinbase_tx.clone());
	// build merkle proof and block identifier and save in wallet
	let output_id = OutputIdentifier::from_output(&coinbase_tx.0.clone());
	let m_proof = chain.get_merkle_proof(&output_id, &chain.head_header().unwrap());
	let block_id = Some(BlockIdentifier(chain.head_header().unwrap().hash()));
	let _ = WalletData::with_wallet(&wallet.0.data_file_dir, |wallet_data| {
		if let Entry::Occupied(mut output) = wallet_data
			.outputs
			.entry(fees.key_id.as_ref().unwrap().to_hex())
		{
			let output = output.get_mut();
			output.block = block_id;
			output.merkle_proof = Some(MerkleProofWrapper(m_proof.unwrap()));
		}
	});
}

/// adds many block rewards to a wallet
fn award_blocks_to_wallet(chain: &Chain, wallet: &(WalletConfig, Keychain), num_rewards: usize) {
	for _ in 0..num_rewards {
		award_block_to_wallet(chain, wallet);
	}
}

fn create_wallet(dir: &str) -> (WalletConfig, Keychain) {
	let mut wallet_config = WalletConfig::default();
	wallet_config.data_file_dir = String::from(dir);
	let wallet_seed = wallet::WalletSeed::init_file(&wallet_config).unwrap();
	let keychain = wallet_seed
		.derive_keychain("")
		.expect("Failed to derive keychain from seed file and passphrase.");
	(wallet_config, keychain)
}

/// Build a transaction between 2 parties
#[cfg(test)]
#[test]
fn build_transaction() {
	let chain = setup("test_output", "build_transaction/.grin");
	let wallet1 = create_wallet("test_output/build_transaction/wallet1");
	let wallet2 = create_wallet("test_output/build_transaction/wallet2");
	award_blocks_to_wallet(&chain, &wallet1, 10);
	// Wallet 1 has 600 Grins, wallet 2 has 0. Create a transaction that sends
	// 300 Grins from wallet 1 to wallet 2, using libwallet
	// Sender creates a new aggsig context
	let mut sender_context_manager = aggsig::ContextManager::new();
	let tx_id = Uuid::new_v4();

	// Get lock height
	let chain_tip = chain.head().unwrap();

	// ensure outputs we're selecting are up to date
	let res = common::refresh_output_state_local(&wallet1.0, &wallet1.1, &chain);

	if let Err(e) = res {
		panic!("Unable to refresh sender wallet outputs");
	}

	let partial_tx = transaction::sender_initiation(
		&wallet1.0,
		&wallet1.1,
		&tx_id,
		&mut sender_context_manager,
		300000000000,
		chain_tip.height,
		3,
		1000,
		true,
	).unwrap();
}
