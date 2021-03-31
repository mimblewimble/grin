// Copyright 2021 The Grin Developers
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
use self::chain::types::Options;
use self::chain::Chain;
use self::core::core::hash::Hashed;
use self::core::core::Block;
use self::core::genesis;
use self::core::global::ChainTypes;
use self::core::libtx::{self, reward};
use self::core::{consensus, global, pow};
use self::keychain::{ExtKeychainPath, Keychain};
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use std::fs;
use std::sync::Arc;

pub fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

pub fn init_chain(dir_name: &str, genesis: Block) -> Chain {
	Chain::init(
		dir_name.to_string(),
		Arc::new(NoopAdapter {}),
		genesis,
		pow::verify_size,
		false,
	)
	.unwrap()
}

/// Build genesis block with reward (non-empty, like we have in mainnet).
pub fn genesis_block<K>(keychain: &K) -> Block
where
	K: Keychain,
{
	let key_id = keychain::ExtKeychain::derive_key_id(0, 1, 0, 0, 0);
	let reward = reward::output(
		keychain,
		&libtx::ProofBuilder::new(keychain),
		&key_id,
		0,
		false,
	)
	.unwrap();

	genesis::genesis_dev().with_reward(reward.0, reward.1)
}

/// Mine a chain of specified length to assist with automated tests.
/// Probably a good idea to call clean_output_dir at the beginning and end of each test.
#[allow(dead_code)]
pub fn mine_chain(dir_name: &str, chain_length: u64) -> Chain {
	global::set_local_chain_type(ChainTypes::AutomatedTesting);
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let genesis = genesis_block(&keychain);
	let mut chain = init_chain(dir_name, genesis.clone());
	mine_some_on_top(&mut chain, chain_length, &keychain);
	chain
}

#[allow(dead_code)]
fn mine_some_on_top<K>(chain: &mut Chain, chain_length: u64, keychain: &K)
where
	K: Keychain,
{
	for n in 1..chain_length {
		let prev = chain.head_header().unwrap();
		let next_header_info =
			consensus::next_difficulty(prev.height + 1, chain.difficulty_iter().unwrap());
		let pk = ExtKeychainPath::new(1, n as u32, 0, 0, 0).to_identifier();
		let reward =
			libtx::reward::output(keychain, &libtx::ProofBuilder::new(keychain), &pk, 0, false)
				.unwrap();
		let mut b =
			core::core::Block::new(&prev, &[], next_header_info.difficulty, reward).unwrap();
		b.header.timestamp = prev.timestamp + Duration::seconds(60);
		b.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		chain.set_txhashset_roots(&mut b).unwrap();

		let edge_bits = global::min_edge_bits();
		b.header.pow.proof.edge_bits = edge_bits;
		pow::pow_size(
			&mut b.header,
			next_header_info.difficulty,
			global::proofsize(),
			edge_bits,
		)
		.unwrap();

		let bhash = b.hash();
		chain.process_block(b, Options::MINE).unwrap();

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
