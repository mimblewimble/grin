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

//! Core tests
extern crate grin_core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

use std::sync::{Arc, RwLock};

pub mod common;

use common::{new_block, tx1i1o, tx1i2o, tx2i1o};
use grin_core::core::block::BlockHeader;
use grin_core::core::block::Error::KernelLockHeight;
use grin_core::core::hash::{Hashed, ZERO_HASH};
use grin_core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use grin_core::core::{aggregate, deaggregate, KernelFeatures, Output, Transaction};
use grin_core::ser;
use keychain::{BlindingFactor, ExtKeychain, Keychain};
use util::static_secp_instance;
use wallet::libtx::build::{
	self, initial_tx, input, output, with_excess, with_fee, with_lock_height,
};

#[test]
fn simple_tx_ser() {
	let tx = tx2i1o();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &tx).expect("serialization failed");
	let target_len = 955;
	assert_eq!(vec.len(), target_len,);
}

#[test]
fn simple_tx_ser_deser() {
	let tx = tx2i1o();
	let mut vec = Vec::new();
	ser::serialize(&mut vec, &tx).expect("serialization failed");
	let dtx: Transaction = ser::deserialize(&mut &vec[..]).unwrap();
	assert_eq!(dtx.fee(), 2);
	assert_eq!(dtx.inputs().len(), 2);
	assert_eq!(dtx.outputs().len(), 1);
	assert_eq!(tx.hash(), dtx.hash());
}

#[test]
fn tx_double_ser_deser() {
	// checks serializing doesn't mess up the tx and produces consistent results
	let btx = tx2i1o();

	let mut vec = Vec::new();
	assert!(ser::serialize(&mut vec, &btx).is_ok());
	let dtx: Transaction = ser::deserialize(&mut &vec[..]).unwrap();

	let mut vec2 = Vec::new();
	assert!(ser::serialize(&mut vec2, &btx).is_ok());
	let dtx2: Transaction = ser::deserialize(&mut &vec2[..]).unwrap();

	assert_eq!(btx.hash(), dtx.hash());
	assert_eq!(dtx.hash(), dtx2.hash());
}

#[test]
#[should_panic(expected = "InvalidSecretKey")]
fn test_zero_commit_fails() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	// blinding should fail as signing with a zero r*G shouldn't work
	build::transaction(
		vec![
			input(10, key_id1.clone()),
			output(9, key_id1.clone()),
			with_fee(1),
		],
		&keychain,
	).unwrap();
}

fn verifier_cache() -> Arc<RwLock<VerifierCache>> {
	Arc::new(RwLock::new(LruVerifierCache::new()))
}

#[test]
fn build_tx_kernel() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	// first build a valid tx with corresponding blinding factor
	let tx = build::transaction(
		vec![
			input(10, key_id1),
			output(5, key_id2),
			output(3, key_id3),
			with_fee(2),
		],
		&keychain,
	).unwrap();

	// check the tx is valid
	tx.validate(verifier_cache()).unwrap();

	// check the kernel is also itself valid
	assert_eq!(tx.kernels().len(), 1);
	let kern = &tx.kernels()[0];
	kern.verify().unwrap();

	assert_eq!(kern.features, KernelFeatures::DEFAULT_KERNEL);
	assert_eq!(kern.fee, tx.fee());
}

// Combine two transactions into one big transaction (with multiple kernels)
// and check it still validates.
#[test]
fn transaction_cut_through() {
	let tx1 = tx1i2o();
	let tx2 = tx2i1o();

	assert!(tx1.validate(verifier_cache()).is_ok());
	assert!(tx2.validate(verifier_cache()).is_ok());

	let vc = verifier_cache();

	// now build a "cut_through" tx from tx1 and tx2
	let tx3 = aggregate(vec![tx1, tx2]).unwrap();

	assert!(tx3.validate(vc.clone()).is_ok());
}

// Attempt to deaggregate a multi-kernel transaction in a different way
#[test]
fn multi_kernel_transaction_deaggregation() {
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();
	let tx4 = tx1i1o();

	let vc = verifier_cache();

	assert!(tx1.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());
	assert!(tx3.validate(vc.clone()).is_ok());
	assert!(tx4.validate(vc.clone()).is_ok());

	let tx1234 = aggregate(vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()]).unwrap();
	let tx12 = aggregate(vec![tx1.clone(), tx2.clone()]).unwrap();
	let tx34 = aggregate(vec![tx3.clone(), tx4.clone()]).unwrap();

	assert!(tx1234.validate(vc.clone()).is_ok());
	assert!(tx12.validate(vc.clone()).is_ok());
	assert!(tx34.validate(vc.clone()).is_ok());

	let deaggregated_tx34 = deaggregate(tx1234.clone(), vec![tx12.clone()]).unwrap();
	assert!(deaggregated_tx34.validate(vc.clone()).is_ok());
	assert_eq!(tx34, deaggregated_tx34);

	let deaggregated_tx12 = deaggregate(tx1234.clone(), vec![tx34.clone()]).unwrap();

	assert!(deaggregated_tx12.validate(vc.clone()).is_ok());
	assert_eq!(tx12, deaggregated_tx12);
}

#[test]
fn multi_kernel_transaction_deaggregation_2() {
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();

	let vc = verifier_cache();

	assert!(tx1.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());
	assert!(tx3.validate(vc.clone()).is_ok());

	let tx123 = aggregate(vec![tx1.clone(), tx2.clone(), tx3.clone()]).unwrap();
	let tx12 = aggregate(vec![tx1.clone(), tx2.clone()]).unwrap();

	assert!(tx123.validate(vc.clone()).is_ok());
	assert!(tx12.validate(vc.clone()).is_ok());

	let deaggregated_tx3 = deaggregate(tx123.clone(), vec![tx12.clone()]).unwrap();
	assert!(deaggregated_tx3.validate(vc.clone()).is_ok());
	assert_eq!(tx3, deaggregated_tx3);
}

#[test]
fn multi_kernel_transaction_deaggregation_3() {
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();

	let vc = verifier_cache();

	assert!(tx1.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());
	assert!(tx3.validate(vc.clone()).is_ok());

	let tx123 = aggregate(vec![tx1.clone(), tx2.clone(), tx3.clone()]).unwrap();
	let tx13 = aggregate(vec![tx1.clone(), tx3.clone()]).unwrap();
	let tx2 = aggregate(vec![tx2.clone()]).unwrap();

	assert!(tx123.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());

	let deaggregated_tx13 = deaggregate(tx123.clone(), vec![tx2.clone()]).unwrap();
	assert!(deaggregated_tx13.validate(vc.clone()).is_ok());
	assert_eq!(tx13, deaggregated_tx13);
}

#[test]
fn multi_kernel_transaction_deaggregation_4() {
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();
	let tx4 = tx1i1o();
	let tx5 = tx1i1o();

	let vc = verifier_cache();

	assert!(tx1.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());
	assert!(tx3.validate(vc.clone()).is_ok());
	assert!(tx4.validate(vc.clone()).is_ok());
	assert!(tx5.validate(vc.clone()).is_ok());

	let tx12345 = aggregate(vec![
		tx1.clone(),
		tx2.clone(),
		tx3.clone(),
		tx4.clone(),
		tx5.clone(),
	]).unwrap();
	assert!(tx12345.validate(vc.clone()).is_ok());

	let deaggregated_tx5 = deaggregate(
		tx12345.clone(),
		vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()],
	).unwrap();
	assert!(deaggregated_tx5.validate(vc.clone()).is_ok());
	assert_eq!(tx5, deaggregated_tx5);
}

#[test]
fn multi_kernel_transaction_deaggregation_5() {
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();
	let tx4 = tx1i1o();
	let tx5 = tx1i1o();

	let vc = verifier_cache();

	assert!(tx1.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());
	assert!(tx3.validate(vc.clone()).is_ok());
	assert!(tx4.validate(vc.clone()).is_ok());
	assert!(tx5.validate(vc.clone()).is_ok());

	let tx12345 = aggregate(vec![
		tx1.clone(),
		tx2.clone(),
		tx3.clone(),
		tx4.clone(),
		tx5.clone(),
	]).unwrap();
	let tx12 = aggregate(vec![tx1.clone(), tx2.clone()]).unwrap();
	let tx34 = aggregate(vec![tx3.clone(), tx4.clone()]).unwrap();

	assert!(tx12345.validate(vc.clone()).is_ok());

	let deaggregated_tx5 = deaggregate(tx12345.clone(), vec![tx12.clone(), tx34.clone()]).unwrap();
	assert!(deaggregated_tx5.validate(vc.clone()).is_ok());
	assert_eq!(tx5, deaggregated_tx5);
}

// Attempt to deaggregate a multi-kernel transaction
#[test]
fn basic_transaction_deaggregation() {
	let tx1 = tx1i2o();
	let tx2 = tx2i1o();

	let vc = verifier_cache();

	assert!(tx1.validate(vc.clone()).is_ok());
	assert!(tx2.validate(vc.clone()).is_ok());

	// now build a "cut_through" tx from tx1 and tx2
	let tx3 = aggregate(vec![tx1.clone(), tx2.clone()]).unwrap();

	assert!(tx3.validate(vc.clone()).is_ok());

	let deaggregated_tx1 = deaggregate(tx3.clone(), vec![tx2.clone()]).unwrap();

	assert!(deaggregated_tx1.validate(vc.clone()).is_ok());
	assert_eq!(tx1, deaggregated_tx1);

	let deaggregated_tx2 = deaggregate(tx3.clone(), vec![tx1.clone()]).unwrap();

	assert!(deaggregated_tx2.validate(vc.clone()).is_ok());
	assert_eq!(tx2, deaggregated_tx2);
}

#[test]
fn hash_output() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx = build::transaction(
		vec![
			input(75, key_id1),
			output(42, key_id2),
			output(32, key_id3),
			with_fee(1),
		],
		&keychain,
	).unwrap();
	let h = tx.outputs()[0].hash();
	assert!(h != ZERO_HASH);
	let h2 = tx.outputs()[1].hash();
	assert!(h != h2);
}

#[ignore]
#[test]
fn blind_tx() {
	let btx = tx2i1o();
	assert!(btx.validate(verifier_cache()).is_ok());

	// Ignored for bullet proofs, because calling range_proof_info
	// with a bullet proof causes painful errors

	// checks that the range proof on our blind output is sufficiently hiding
	let Output { proof, .. } = btx.outputs()[0];

	let secp = static_secp_instance();
	let secp = secp.lock().unwrap();
	let info = secp.range_proof_info(proof);

	assert!(info.min == 0);
	assert!(info.max == u64::max_value());
}

#[test]
fn tx_hash_diff() {
	let btx1 = tx2i1o();
	let btx2 = tx1i1o();

	if btx1.hash() == btx2.hash() {
		panic!("diff txs have same hash")
	}
}

/// Simulate the standard exchange between 2 parties when creating a basic
/// 2 inputs, 2 outputs transaction.
#[test]
fn tx_build_exchange() {
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);
	let key_id4 = ExtKeychain::derive_key_id(1, 4, 0, 0, 0);

	let (tx_alice, blind_sum) = {
		// Alice gets 2 of her pre-existing outputs to send 5 coins to Bob, they
		// become inputs in the new transaction
		let (in1, in2) = (input(4, key_id1), input(3, key_id2));

		// Alice builds her transaction, with change, which also produces the sum
		// of blinding factors before they're obscured.
		let (tx, sum) =
			build::partial_transaction(vec![in1, in2, output(1, key_id3), with_fee(2)], &keychain)
				.unwrap();

		(tx, sum)
	};

	// From now on, Bob only has the obscured transaction and the sum of
	// blinding factors. He adds his output, finalizes the transaction so it's
	// ready for broadcast.
	let tx_final = build::transaction(
		vec![
			initial_tx(tx_alice),
			with_excess(blind_sum),
			output(4, key_id4),
		],
		&keychain,
	).unwrap();

	tx_final.validate(verifier_cache()).unwrap();
}

#[test]
fn reward_empty_block() {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let previous_header = BlockHeader::default();

	let b = new_block(vec![], &keychain, &previous_header, &key_id);

	b.cut_through()
		.unwrap()
		.validate(&BlindingFactor::zero(), verifier_cache())
		.unwrap();
}

#[test]
fn reward_with_tx_block() {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let vc = verifier_cache();

	let mut tx1 = tx2i1o();
	tx1.validate(vc.clone()).unwrap();

	let previous_header = BlockHeader::default();

	let block = new_block(vec![&mut tx1], &keychain, &previous_header, &key_id);
	block
		.cut_through()
		.unwrap()
		.validate(&BlindingFactor::zero(), vc.clone())
		.unwrap();
}

#[test]
fn simple_block() {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let vc = verifier_cache();

	let mut tx1 = tx2i1o();
	let mut tx2 = tx1i1o();

	let previous_header = BlockHeader::default();
	let b = new_block(
		vec![&mut tx1, &mut tx2],
		&keychain,
		&previous_header,
		&key_id,
	);

	b.validate(&BlindingFactor::zero(), vc.clone()).unwrap();
}

#[test]
fn test_block_with_timelocked_tx() {
	let keychain = keychain::ExtKeychain::from_random_seed().unwrap();

	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let vc = verifier_cache();

	// first check we can add a timelocked tx where lock height matches current
	// block height and that the resulting block is valid
	let tx1 = build::transaction(
		vec![
			input(5, key_id1.clone()),
			output(3, key_id2.clone()),
			with_fee(2),
			with_lock_height(1),
		],
		&keychain,
	).unwrap();

	let previous_header = BlockHeader::default();

	let b = new_block(vec![&tx1], &keychain, &previous_header, &key_id3.clone());
	b.validate(&BlindingFactor::zero(), vc.clone()).unwrap();

	// now try adding a timelocked tx where lock height is greater than current
	// block height
	let tx1 = build::transaction(
		vec![
			input(5, key_id1.clone()),
			output(3, key_id2.clone()),
			with_fee(2),
			with_lock_height(2),
		],
		&keychain,
	).unwrap();

	let previous_header = BlockHeader::default();
	let b = new_block(vec![&tx1], &keychain, &previous_header, &key_id3.clone());

	match b.validate(&BlindingFactor::zero(), vc.clone()) {
		Err(KernelLockHeight(height)) => {
			assert_eq!(height, 2);
		}
		_ => panic!("expecting KernelLockHeight error here"),
	}
}

#[test]
pub fn test_verify_1i1o_sig() {
	let tx = tx1i1o();
	tx.validate(verifier_cache()).unwrap();
}

#[test]
pub fn test_verify_2i1o_sig() {
	let tx = tx2i1o();
	tx.validate(verifier_cache()).unwrap();
}
