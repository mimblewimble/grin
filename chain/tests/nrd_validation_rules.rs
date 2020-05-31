// Copyright 2020 The Grin Developers
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
use crate::chain::{Chain, Error, Options};
use crate::core::core::{Block, KernelFeatures, NRDRelativeHeight, Transaction, TxKernel};
use crate::core::libtx::{aggsig, build, reward, ProofBuilder};
use crate::core::{consensus, global, pow};
use crate::keychain::{BlindingFactor, ExtKeychain, ExtKeychainPath, Identifier, Keychain};
use chrono::Duration;

fn build_block<K>(
	chain: &Chain,
	keychain: &K,
	key_id: &Identifier,
	txs: Vec<Transaction>,
) -> Result<Block, Error>
where
	K: Keychain,
{
	// Tests need to build "invalid" blocks so disable NRD feature flag temprorarily.
	let is_nrd_enabled = global::is_nrd_enabled();
	global::set_local_nrd_enabled(false);

	let prev = chain.head_header()?;
	let next_header_info = consensus::next_difficulty(1, chain.difficulty_iter().unwrap());
	let fee = txs.iter().map(|x| x.fee()).sum();
	let reward =
		reward::output(keychain, &ProofBuilder::new(keychain), key_id, fee, false).unwrap();

	let mut block = Block::new(&prev, txs, next_header_info.clone().difficulty, reward)?;

	block.header.timestamp = prev.timestamp + Duration::seconds(60);
	block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

	chain.set_txhashset_roots(&mut block)?;

	let edge_bits = global::min_edge_bits();
	block.header.pow.proof.edge_bits = edge_bits;
	pow::pow_size(
		&mut block.header,
		next_header_info.difficulty,
		global::proofsize(),
		edge_bits,
	)
	.unwrap();

	// Restore NRD feature flag after building the potentially "invalid" block.
	global::set_local_nrd_enabled(is_nrd_enabled);

	Ok(block)
}

#[test]
fn process_block_nrd_validation_rules() -> Result<(), Error> {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_nrd_enabled(true);

	util::init_test_logger();

	let chain_dir = ".grin.nrd_kernel";
	clean_output_dir(chain_dir);

	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let genesis = genesis_block(&keychain);
	let chain = init_chain(chain_dir, genesis.clone());

	for n in 1..9 {
		let key_id = ExtKeychainPath::new(1, n, 0, 0, 0).to_identifier();
		let block = build_block(&chain, &keychain, &key_id, vec![])?;
		chain.process_block(block, Options::MINE).unwrap();
	}

	assert_eq!(chain.head().unwrap().height, 8);

	// TODO - build 2 "half txs" with shared NRD kernel and locked with relative_height = 2
	// Check invalid if tx1 and tx2 included in same block.
	// Check invalid if tx2 included in next block.
	// Check valid if tx2 included in subsequent block (height diff at least 2).

	let mut kernel = TxKernel::with_features(KernelFeatures::NoRecentDuplicate {
		fee: 20000,
		relative_height: NRDRelativeHeight::new(2)?,
	});

	// // Construct the message to be signed.
	let msg = kernel.msg_to_sign().unwrap();

	// // Generate a kernel with public excess and associated signature.
	let excess = BlindingFactor::rand(&keychain.secp());
	let skey = excess.secret_key(&keychain.secp()).unwrap();
	kernel.excess = keychain.secp().commit(0, skey).unwrap();
	let pubkey = &kernel.excess.to_pubkey(&keychain.secp()).unwrap();
	kernel.excess_sig =
		aggsig::sign_with_blinding(&keychain.secp(), &msg, &excess, Some(&pubkey)).unwrap();
	kernel.verify().unwrap();

	let key_id1 = ExtKeychainPath::new(1, 1, 0, 0, 0).to_identifier();
	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
	let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();

	let tx1 = build::transaction_with_kernel(
		vec![
			build::coinbase_input(consensus::REWARD, key_id1.clone()),
			build::output(consensus::REWARD - 20000, key_id2.clone()),
		],
		kernel.clone(),
		excess.clone(),
		&keychain,
		&builder,
	)
	.unwrap();

	let tx2 = build::transaction_with_kernel(
		vec![
			build::input(consensus::REWARD - 20000, key_id2.clone()),
			build::output(consensus::REWARD - 40000, key_id3.clone()),
		],
		kernel.clone(),
		excess.clone(),
		&keychain,
		&builder,
	)
	.unwrap();

	let key_id9 = ExtKeychainPath::new(1, 9, 0, 0, 0).to_identifier();

	// Check block containing both tx1 and tx2 is invalid.
	let block = build_block(&chain, &keychain, &key_id9, vec![tx1, tx2])?;
	assert!(chain.process_block(block, Options::MINE).is_err());

	panic!("tbc");

	// chain.process_block(block, Options::MINE).unwrap();
	// chain.validate(false).unwrap();

	clean_output_dir(chain_dir);
	Ok(())
}
