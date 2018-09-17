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
extern crate grin_core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

use std::sync::{Arc, RwLock};
use std::time::Instant;

pub mod common;

use common::{new_block, tx1i2o, tx2i1o, txspend1i1o};
use grin_core::consensus::{self, BLOCK_OUTPUT_WEIGHT, MAX_BLOCK_WEIGHT};
use grin_core::core::block::Error;
use grin_core::core::hash::Hashed;
use grin_core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use grin_core::core::Committed;
use grin_core::core::{Block, BlockHeader, KernelFeatures, OutputFeatures};
use grin_core::{global, ser};
use keychain::{BlindingFactor, ExtKeychain, Keychain};
use util::{secp, secp_static};
use wallet::libtx::build::{self, input, output, with_fee};

fn verifier_cache() -> Arc<RwLock<VerifierCache>> {
	Arc::new(RwLock::new(LruVerifierCache::new()))
}

// Too slow for now #[test]
// TODO: make this fast enough or add similar but faster test?
#[allow(dead_code)]
fn too_large_block() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let max_out = MAX_BLOCK_WEIGHT / BLOCK_OUTPUT_WEIGHT;

	let zero_commit = secp_static::commit_to_zero_value();

	let mut pks = vec![];
	for n in 0..(max_out + 1) {
		pks.push(keychain.derive_key_id(n as u32).unwrap());
	}

	let mut parts = vec![];
	for _ in 0..max_out {
		parts.push(output(5, pks.pop().unwrap()));
	}

	let now = Instant::now();
	parts.append(&mut vec![input(500000, pks.pop().unwrap()), with_fee(2)]);
	let tx = build::transaction(parts, &keychain).unwrap();
	println!("Build tx: {}", now.elapsed().as_secs());

	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![&tx], &keychain, &prev, &key_id);
	assert!(
		b.validate(&BlindingFactor::zero(), &zero_commit, verifier_cache())
			.is_err()
	);
}

#[test]
// block with no inputs/outputs/kernels
// no fees, no reward, no coinbase
fn very_empty_block() {
	let b = Block::with_header(BlockHeader::default());

	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);
}

#[test]
// builds a block with a tx spending another and check that cut_through occurred
fn block_with_cut_through() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();

	let zero_commit = secp_static::commit_to_zero_value();

	let mut btx1 = tx2i1o();
	let mut btx2 = build::transaction(
		vec![input(7, key_id1), output(5, key_id2.clone()), with_fee(2)],
		&keychain,
	).unwrap();

	// spending tx2 - reuse key_id2

	let mut btx3 = txspend1i1o(5, &keychain, key_id2.clone(), key_id3);
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(
		vec![&mut btx1, &mut btx2, &mut btx3],
		&keychain,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	println!("3");
	b.validate(&BlindingFactor::zero(), &zero_commit, verifier_cache())
		.unwrap();
	assert_eq!(b.inputs().len(), 3);
	assert_eq!(b.outputs().len(), 3);
	println!("4");
}

#[test]
fn empty_block_with_coinbase_is_valid() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let zero_commit = secp_static::commit_to_zero_value();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![], &keychain, &prev, &key_id);

	assert_eq!(b.inputs().len(), 0);
	assert_eq!(b.outputs().len(), 1);
	assert_eq!(b.kernels().len(), 1);

	let coinbase_outputs = b
		.outputs()
		.iter()
		.filter(|out| out.features.contains(OutputFeatures::COINBASE_OUTPUT))
		.map(|o| o.clone())
		.collect::<Vec<_>>();
	assert_eq!(coinbase_outputs.len(), 1);

	let coinbase_kernels = b
		.kernels()
		.iter()
		.filter(|out| out.features.contains(KernelFeatures::COINBASE_KERNEL))
		.map(|o| o.clone())
		.collect::<Vec<_>>();
	assert_eq!(coinbase_kernels.len(), 1);

	// the block should be valid here (single coinbase output with corresponding
	// txn kernel)
	assert!(
		b.validate(&BlindingFactor::zero(), &zero_commit, verifier_cache())
			.is_ok()
	);
}

#[test]
// test that flipping the COINBASE_OUTPUT flag on the output features
// invalidates the block and specifically it causes verify_coinbase to fail
// additionally verifying the merkle_inputs_outputs also fails
fn remove_coinbase_output_flag() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let zero_commit = secp_static::commit_to_zero_value();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let mut b = new_block(vec![], &keychain, &prev, &key_id);

	assert!(
		b.outputs()[0]
			.features
			.contains(OutputFeatures::COINBASE_OUTPUT)
	);
	b.outputs_mut()[0]
		.features
		.remove(OutputFeatures::COINBASE_OUTPUT);

	assert_eq!(b.verify_coinbase(), Err(Error::CoinbaseSumMismatch));
	assert!(
		b.verify_kernel_sums(b.header.overage(), b.header.total_kernel_offset())
			.is_ok()
	);
	assert_eq!(
		b.validate(&BlindingFactor::zero(), &zero_commit, verifier_cache()),
		Err(Error::CoinbaseSumMismatch)
	);
}

#[test]
// test that flipping the COINBASE_KERNEL flag on the kernel features
// invalidates the block and specifically it causes verify_coinbase to fail
fn remove_coinbase_kernel_flag() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let zero_commit = secp_static::commit_to_zero_value();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let mut b = new_block(vec![], &keychain, &prev, &key_id);

	assert!(
		b.kernels()[0]
			.features
			.contains(KernelFeatures::COINBASE_KERNEL)
	);
	b.kernels_mut()[0]
		.features
		.remove(KernelFeatures::COINBASE_KERNEL);

	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);

	assert_eq!(
		b.validate(&BlindingFactor::zero(), &zero_commit, verifier_cache()),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);
}

#[test]
fn serialize_deserialize_block_header() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let header1 = b.header;

	let mut vec = Vec::new();
	ser::serialize(&mut vec, &header1).expect("serialization failed");
	let header2: BlockHeader = ser::deserialize(&mut &vec[..]).unwrap();

	assert_eq!(header1.hash(), header2.hash());
	assert_eq!(header1, header2);
}

#[test]
fn serialize_deserialize_block() {
	let tx1 = tx1i2o();
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);

	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let b2: Block = ser::deserialize(&mut &vec[..]).unwrap();

	assert_eq!(b.hash(), b2.hash());
	assert_eq!(b.header, b2.header);
	assert_eq!(b.inputs(), b2.inputs());
	assert_eq!(b.outputs(), b2.outputs());
	assert_eq!(b.kernels(), b2.kernels());
}

#[test]
fn empty_block_serialized_size() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 1_252;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn block_single_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 2_834;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn block_10_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	global::set_mining_mode(global::ChainTypes::Mainnet);

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(txs.iter().collect(), &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 17_072;
	assert_eq!(vec.len(), target_len,);
}

#[test]
fn empty_block_v2_switch() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let mut prev = BlockHeader::default();
	prev.height = consensus::HEADER_V2_HARD_FORK - 1;
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 1_260;
	assert_eq!(b.header.version, 2);
	assert_eq!(vec.len(), target_len);

	// another try right before v2
	prev.height = consensus::HEADER_V2_HARD_FORK - 2;
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 1_252;
	assert_eq!(b.header.version, 1);
	assert_eq!(vec.len(), target_len);
}
