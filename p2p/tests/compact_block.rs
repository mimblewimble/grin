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
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

use chrono::Duration;
use core::core::hash::Hashed;
use core::core::id::ShortIdentifiable;
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader, KernelFeatures, Transaction};
use core::ser;
use keychain::{ExtKeychain, Identifier, Keychain};
use p2p::compact_block::{self, CompactBlock};
use wallet::libtx::build::{self, input, output, with_fee};
use wallet::libtx::reward;

// utility to create a block without worrying about the key or previous
// header
pub fn new_block<K>(
	txs: Vec<&Transaction>,
	keychain: &K,
	previous_header: &BlockHeader,
	key_id: &Identifier,
) -> Block
where
	K: Keychain,
{
	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let reward_output = reward::output(keychain, &key_id, fees, previous_header.height).unwrap();
	Block::new(
		&previous_header,
		txs.into_iter().cloned().collect(),
		Difficulty::one(),
		reward_output,
	).unwrap()
}

// utility producing a transaction with a single input
// and two outputs (one change output)
// Note: this tx has an "offset" kernel
pub fn tx1i2o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();

	build::transaction(
		vec![
			input(6, key_id1),
			output(3, key_id2),
			output(1, key_id3),
			with_fee(2),
		],
		&keychain,
	).unwrap()
}

#[test]
fn empty_compact_block_serialized_size() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_260;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn compact_block_single_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_266;
	assert_eq!(vec.len(), target_len);
}

#[test]
fn compact_block_10_tx_serialized_size() {
	let keychain = ExtKeychain::from_random_seed().unwrap();

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(txs.iter().collect(), &keychain, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &cb).expect("serialization failed");
	let target_len = 1_320;
	assert_eq!(vec.len(), target_len,);
}

#[test]
fn compact_block_hash_with_nonce() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let tx = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
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
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![&tx1], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();

	assert_eq!(cb.out_full().len(), 1);
	assert_eq!(cb.kern_full().len(), 1);
	assert_eq!(cb.kern_ids().len(), 1);

	assert_eq!(
		cb.kern_ids()[0],
		b.kernels()
			.iter()
			.find(|x| !x.features.contains(KernelFeatures::COINBASE_KERNEL))
			.unwrap()
			.short_id(&cb.hash(), cb.nonce)
	);
}

#[test]
fn hydrate_empty_compact_block() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
	let b = new_block(vec![], &keychain, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();
	let hb = compact_block::hydrate_block(cb, vec![]).unwrap();
	assert_eq!(hb.header, b.header);
	assert_eq!(hb.outputs(), b.outputs());
	assert_eq!(hb.kernels(), b.kernels());
}

#[test]
fn serialize_deserialize_compact_block() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = keychain.derive_key_id(1).unwrap();
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
