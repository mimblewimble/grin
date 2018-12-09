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

//! Common test functions

use crate::core::core::{
	block::{Block, BlockHeader},
	Transaction,
};
use crate::core::libtx::{
	build::{self, input, output, with_fee},
	reward,
};
use crate::core::pow::Difficulty;
use crate::keychain::{Identifier, Keychain};
use grin_core as core;
use grin_keychain as keychain;

// utility producing a transaction with 2 inputs and a single outputs
pub fn tx2i1o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	build::transaction(
		vec![
			input(10, key_id1),
			input(11, key_id2),
			output(19, key_id3),
			with_fee(2),
		],
		&keychain,
	)
	.unwrap()
}

// utility producing a transaction with a single input and output
pub fn tx1i1o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);

	build::transaction(
		vec![input(5, key_id1), output(3, key_id2), with_fee(2)],
		&keychain,
	)
	.unwrap()
}

// utility producing a transaction with a single input
// and two outputs (one change output)
// Note: this tx has an "offset" kernel
pub fn tx1i2o() -> Transaction {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	build::transaction(
		vec![
			input(6, key_id1),
			output(3, key_id2),
			output(1, key_id3),
			with_fee(2),
		],
		&keychain,
	)
	.unwrap()
}

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
		Difficulty::min(),
		reward_output,
	)
	.unwrap()
}

// utility producing a transaction that spends an output with the provided
// value and blinding key
pub fn txspend1i1o<K>(v: u64, keychain: &K, key_id1: Identifier, key_id2: Identifier) -> Transaction
where
	K: Keychain,
{
	build::transaction(
		vec![input(v, key_id1), output(3, key_id2), with_fee(2)],
		keychain,
	)
	.unwrap()
}
