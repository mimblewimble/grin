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

mod chain_test_helper;

use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;

use self::chain_test_helper::{clean_output_dir, genesis_block, init_chain};
use crate::chain::{Chain, Options};
use crate::core::core::{Block, KernelFeatures, NRDRelativeHeight, Transaction};
use crate::core::libtx::{build, reward, ProofBuilder};
use crate::core::{consensus, global, pow};
use crate::keychain::{ExtKeychain, ExtKeychainPath, Identifier, Keychain};
use chrono::Duration;

fn build_block<K>(chain: &Chain, keychain: &K, key_id: &Identifier, txs: Vec<Transaction>) -> Block
where
	K: Keychain,
{
	let prev = chain.head_header().unwrap();
	let next_header_info = consensus::next_difficulty(1, chain.difficulty_iter().unwrap());
	let fee = txs.iter().map(|x| x.fee()).sum();
	let reward =
		reward::output(keychain, &ProofBuilder::new(keychain), key_id, fee, false).unwrap();

	let mut block = Block::new(&prev, &txs, next_header_info.clone().difficulty, reward).unwrap();

	block.header.timestamp = prev.timestamp + Duration::seconds(60);
	block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

	chain.set_txhashset_roots(&mut block).unwrap();

	let edge_bits = global::min_edge_bits();
	block.header.pow.proof.edge_bits = edge_bits;
	pow::pow_size(
		&mut block.header,
		next_header_info.difficulty,
		global::proofsize(),
		edge_bits,
	)
	.unwrap();

	block
}

#[test]
fn mine_block_with_nrd_kernel_and_nrd_feature_enabled() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_nrd_enabled(true);

	util::init_test_logger();

	let chain_dir = ".grin.nrd_kernel";
	clean_output_dir(chain_dir);

	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let pb = ProofBuilder::new(&keychain);
	let genesis = genesis_block(&keychain);
	let chain = init_chain(chain_dir, genesis.clone());

	for n in 1..9 {
		let key_id = ExtKeychainPath::new(1, n, 0, 0, 0).to_identifier();
		let block = build_block(&chain, &keychain, &key_id, vec![]);
		chain.process_block(block, Options::MINE).unwrap();
	}

	assert_eq!(chain.head().unwrap().height, 8);

	let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
	let tx = build::transaction(
		KernelFeatures::NoRecentDuplicate {
			fee: 20000.into(),
			relative_height: NRDRelativeHeight::new(1440).unwrap(),
		},
		&[
			build::coinbase_input(consensus::REWARD, key_id1.clone()),
			build::output(consensus::REWARD - 20000, key_id2.clone()),
		],
		&keychain,
		&pb,
	)
	.unwrap();

	let key_id9 = ExtKeychainPath::new(1, 9, 0, 0, 0).to_identifier();
	let block = build_block(&chain, &keychain, &key_id9, vec![tx]);
	chain.process_block(block, Options::MINE).unwrap();
	chain.validate(false).unwrap();

	clean_output_dir(chain_dir);
}

#[test]
fn mine_invalid_block_with_nrd_kernel_and_nrd_feature_enabled_before_hf() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_nrd_enabled(true);

	util::init_test_logger();

	let chain_dir = ".grin.invalid_nrd_kernel";
	clean_output_dir(chain_dir);

	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let pb = ProofBuilder::new(&keychain);
	let genesis = genesis_block(&keychain);
	let chain = init_chain(chain_dir, genesis.clone());

	for n in 1..8 {
		let key_id = ExtKeychainPath::new(1, n, 0, 0, 0).to_identifier();
		let block = build_block(&chain, &keychain, &key_id, vec![]);
		chain.process_block(block, Options::MINE).unwrap();
	}

	assert_eq!(chain.head().unwrap().height, 7);

	let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
	let tx = build::transaction(
		KernelFeatures::NoRecentDuplicate {
			fee: 20000.into(),
			relative_height: NRDRelativeHeight::new(1440).unwrap(),
		},
		&[
			build::coinbase_input(consensus::REWARD, key_id1.clone()),
			build::output(consensus::REWARD - 20000, key_id2.clone()),
		],
		&keychain,
		&pb,
	)
	.unwrap();

	let key_id8 = ExtKeychainPath::new(1, 8, 0, 0, 0).to_identifier();
	let block = build_block(&chain, &keychain, &key_id8, vec![tx]);
	let res = chain.process_block(block, Options::MINE);
	assert!(res.is_err());
	clean_output_dir(chain_dir);
}
