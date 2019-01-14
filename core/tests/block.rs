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

pub mod common;
use crate::common::{new_block, tx1i2o, tx2i1o, txspend1i1o};
use crate::core::consensus::{BLOCK_OUTPUT_WEIGHT, MAX_BLOCK_WEIGHT};
use crate::core::core::block::Error;
use crate::core::core::hash::Hashed;
use crate::core::core::id::ShortIdentifiable;
use crate::core::core::transaction::{self, Transaction};
use crate::core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use crate::core::core::Committed;
use crate::core::core::{Block, BlockHeader, CompactBlock, KernelFeatures, OutputFeatures};
use crate::core::libtx::build::{self, input, output, with_fee};
use crate::core::{global, ser};
use crate::keychain::{BlindingFactor, ExtKeychain, Keychain};
use crate::util::secp;
use crate::util::RwLock;
use chrono::Duration;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::sync::Arc;
use std::time::Instant;

fn verifier_cache() -> Arc<RwLock<dyn VerifierCache>> {
	Arc::new(RwLock::new(LruVerifierCache::new()))
}

// Too slow for now #[test]
// TODO: make this fast enough or add similar but faster test?
#[allow(dead_code)]
fn too_large_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let max_out = MAX_BLOCK_WEIGHT / BLOCK_OUTPUT_WEIGHT;

	let mut pks = vec![];
	for n in 0..(max_out + 1) {
		pks.push(ExtKeychain::derive_key_id(1, n as u32, 0, 0, 0));
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
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx], &keychain, &prev, &key_id);
	assert!(b
		.validate(&BlindingFactor::zero(), verifier_cache())
		.is_err());
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
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let mut btx1 = tx2i1o();
	let mut btx2 = build::transaction(
		vec![input(7, key_id1), output(5, key_id2.clone()), with_fee(2)],
		&keychain,
	)
	.unwrap();

	// spending tx2 - reuse key_id2

	let mut btx3 = txspend1i1o(5, &keychain, key_id2.clone(), key_id3);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(
		vec![&mut btx1, &mut btx2, &mut btx3],
		&keychain,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	b.validate(&BlindingFactor::zero(), verifier_cache())
		.unwrap();
	assert_eq!(b.inputs().len(), 3);
	assert_eq!(b.outputs().len(), 3);
}

#[test]
fn empty_block_with_coinbase_is_valid() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &prev, &key_id);

	assert_eq!(b.inputs().len(), 0);
	assert_eq!(b.outputs().len(), 1);
	assert_eq!(b.kernels().len(), 1);

	let coinbase_outputs = b
		.outputs()
		.iter()
		.filter(|out| out.is_coinbase())
		.map(|o| o.clone())
		.collect::<Vec<_>>();
	assert_eq!(coinbase_outputs.len(), 1);

	let coinbase_kernels = b
		.kernels()
		.iter()
		.filter(|out| out.is_coinbase())
		.map(|o| o.clone())
		.collect::<Vec<_>>();
	assert_eq!(coinbase_kernels.len(), 1);

	// the block should be valid here (single coinbase output with corresponding
	// txn kernel)
	assert!(b
		.validate(&BlindingFactor::zero(), verifier_cache())
		.is_ok());
}

#[test]
// test that flipping the COINBASE flag on the output features
// invalidates the block and specifically it causes verify_coinbase to fail
// additionally verifying the merkle_inputs_outputs also fails
fn remove_coinbase_output_flag() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let mut b = new_block(vec![], &keychain, &prev, &key_id);

	assert!(b.outputs()[0].is_coinbase());
	b.outputs_mut()[0].features = OutputFeatures::Plain;

	assert_eq!(b.verify_coinbase(), Err(Error::CoinbaseSumMismatch));
	assert!(b
		.verify_kernel_sums(b.header.overage(), b.header.total_kernel_offset())
		.is_ok());
	assert_eq!(
		b.validate(&BlindingFactor::zero(), verifier_cache()),
		Err(Error::CoinbaseSumMismatch)
	);
}

#[test]
// test that flipping the COINBASE flag on the kernel features
// invalidates the block and specifically it causes verify_coinbase to fail
fn remove_coinbase_kernel_flag() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let mut b = new_block(vec![], &keychain, &prev, &key_id);

	assert!(b.kernels()[0].is_coinbase());
	b.kernels_mut()[0].features = KernelFeatures::Plain;

	// Flipping the coinbase flag results in kernels not summing correctly.
	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);

	// Also results in the block no longer validating correctly
	// because the message being signed on each tx kernel includes the kernel features.
	assert_eq!(
		b.validate(&BlindingFactor::zero(), verifier_cache()),
		Err(Error::Transaction(transaction::Error::IncorrectSignature))
	);
}

#[test]
fn serialize_deserialize_block_header() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
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
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
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
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 1_265;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn block_single_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 2_847;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn empty_compact_block_serialized_size() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_273;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn compact_block_single_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_279;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn block_10_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	global::set_mining_mode(global::ChainTypes::Mainnet);

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(txs.iter().collect(), &keychain, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &b).expect("serialization failed");
	let target_len = 17_085;
	assert_eq!(vec.len(), target_len,);
}

#[test]
fn compact_block_10_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(txs.iter().collect(), &keychain, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_333;
	assert_eq!(vec.len(), target_len,);
}

#[test]
fn compact_block_hash_with_nonce() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let tx = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx], &keychain, &prev, &key_id);
	let cb1: CompactBlock = b.clone().into();
	let cb2: CompactBlock = b.clone().into();

	// random nonce will not affect the hash of the compact block itself
	// hash is based on header POW only
	assert!(cb1.nonce != cb2.nonce);
	assert_eq!(b.hash(), cb1.hash());
	assert_eq!(cb1.hash(), cb2.hash());

	assert!(cb1.kern_ids()[0] != cb2.kern_ids()[0]);

	// check we can identify the specified kernel from the short_id
	// correctly in both of the compact_blocks
	assert_eq!(
		cb1.kern_ids()[0],
		tx.kernels()[0].short_id(&cb1.hash(), cb1.nonce)
	);
	assert_eq!(
		cb2.kern_ids()[0],
		tx.kernels()[0].short_id(&cb2.hash(), cb2.nonce)
	);
}

#[test]
fn convert_block_to_compact_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();

	assert_eq!(cb.out_full().len(), 1);
	assert_eq!(cb.kern_full().len(), 1);
	assert_eq!(cb.kern_ids().len(), 1);

	assert_eq!(
		cb.kern_ids()[0],
		b.kernels()
			.iter()
			.find(|x| !x.is_coinbase())
			.unwrap()
			.short_id(&cb.hash(), cb.nonce)
	);
}

#[test]
fn hydrate_empty_compact_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();
	let hb = Block::hydrate_from(cb, vec![]).unwrap();
	assert_eq!(hb.header, b.header);
	assert_eq!(hb.outputs(), b.outputs());
	assert_eq!(hb.kernels(), b.kernels());
}

#[test]
fn serialize_deserialize_compact_block() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);

	let mut cb1: CompactBlock = b.into();

	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb1).expect("serialization failed");

	// After header serialization, timestamp will lose 'nanos' info, that's the designed behavior.
	// To suppress 'nanos' difference caused assertion fail, we force b.header also lose 'nanos'.
	let origin_ts = cb1.header.timestamp;
	cb1.header.timestamp =
		origin_ts - Duration::nanoseconds(origin_ts.timestamp_subsec_nanos() as i64);

	let cb2: CompactBlock = ser::deserialize(&mut &vec[..]).unwrap();

	assert_eq!(cb1.header, cb2.header);
	assert_eq!(cb1.kern_ids(), cb2.kern_ids());
}

// Duplicate a range proof from a valid output into another of the same amount
#[test]
fn same_amount_outputs_copy_range_proof() {
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx = build::transaction(
		vec![
			input(7, key_id1),
			output(3, key_id2),
			output(3, key_id3),
			with_fee(1),
		],
		&keychain,
	)
	.unwrap();

	// now we reconstruct the transaction, swapping the rangeproofs so they
	// have the wrong privkey
	let ins = tx.inputs();
	let mut outs = tx.outputs().clone();
	let kernels = tx.kernels();
	outs[0].proof = outs[1].proof;

	let key_id = keychain::ExtKeychain::derive_key_id(1, 4, 0, 0, 0);
	let prev = BlockHeader::default();
	let b = new_block(
		vec![&mut Transaction::new(
			ins.clone(),
			outs.clone(),
			kernels.clone(),
		)],
		&keychain,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	match b.validate(&BlindingFactor::zero(), verifier_cache()) {
		Err(Error::Transaction(transaction::Error::Secp(secp::Error::InvalidRangeProof))) => {}
		_ => panic!("Bad range proof should be invalid"),
	}
}

// Swap a range proof with the right private key but wrong amount
#[test]
fn wrong_amount_range_proof() {
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx1 = build::transaction(
		vec![
			input(7, key_id1.clone()),
			output(3, key_id2.clone()),
			output(3, key_id3.clone()),
			with_fee(1),
		],
		&keychain,
	)
	.unwrap();
	let tx2 = build::transaction(
		vec![
			input(7, key_id1),
			output(2, key_id2),
			output(4, key_id3),
			with_fee(1),
		],
		&keychain,
	)
	.unwrap();

	// we take the range proofs from tx2 into tx1 and rebuild the transaction
	let ins = tx1.inputs();
	let mut outs = tx1.outputs().clone();
	let kernels = tx1.kernels();
	outs[0].proof = tx2.outputs()[0].proof;
	outs[1].proof = tx2.outputs()[1].proof;

	let key_id = keychain::ExtKeychain::derive_key_id(1, 4, 0, 0, 0);
	let prev = BlockHeader::default();
	let b = new_block(
		vec![&mut Transaction::new(
			ins.clone(),
			outs.clone(),
			kernels.clone(),
		)],
		&keychain,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	match b.validate(&BlindingFactor::zero(), verifier_cache()) {
		Err(Error::Transaction(transaction::Error::Secp(secp::Error::InvalidRangeProof))) => {}
		_ => panic!("Bad range proof should be invalid"),
	}
}
