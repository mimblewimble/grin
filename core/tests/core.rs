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

//! Core tests

pub mod common;

use self::core::core::block::BlockHeader;
use self::core::core::block::Error::KernelLockHeight;
use self::core::core::hash::{Hashed, ZERO_HASH};
use self::core::core::{
	aggregate, deaggregate, FeeFields, KernelFeatures, Output, OutputFeatures, OutputIdentifier,
	Transaction, TxKernel, Weighting,
};
use self::core::libtx::build::{self, initial_tx, input, output, with_excess};
use self::core::libtx::{aggsig, ProofBuilder};
use self::core::{global, ser};
use crate::common::{new_block, tx1i1o, tx1i2o, tx2i1o};
use grin_core as core;
use keychain::{BlindingFactor, ExtKeychain, Keychain};
use util::static_secp_instance;

// Setup test with AutomatedTesting chain_type;
fn test_setup() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
}

#[test]
fn simple_tx_ser() {
	let tx = tx2i1o();

	// Default protocol version (3).
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &tx).expect("serialization failed");
	assert_eq!(vec.len(), 945);

	// Explicit protocol version 3.
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(3), &tx).expect("serialization failed");
	assert_eq!(vec.len(), 945);

	// We need to convert the tx to v2 compatibility with "features and commitment" inputs
	// to serialize to any previous protocol version.
	// Normally we would do this conversion against the utxo and txpool but we fake it here for testing.
	let inputs: Vec<_> = tx.inputs().into();
	let inputs: Vec<_> = inputs
		.iter()
		.map(|input| OutputIdentifier {
			features: OutputFeatures::Plain,
			commit: input.commitment(),
		})
		.collect();
	let tx = Transaction {
		body: tx.body.replace_inputs(inputs.as_slice().into()),
		..tx
	};

	// Explicit protocol version 1.
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(1), &tx).expect("serialization failed");
	assert_eq!(vec.len(), 955);

	// Explicit protocol version 2.
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(2), &tx).expect("serialization failed");
	assert_eq!(vec.len(), 947);

	// Check we can still serialize to protocol version 3 without explicitly converting the tx.
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(3), &tx).expect("serialization failed");
	assert_eq!(vec.len(), 945);

	// And default protocol version for completeness.
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &tx).expect("serialization failed");
	assert_eq!(vec.len(), 945);
}

#[test]
fn simple_tx_ser_deser() {
	test_setup();
	let tx = tx2i1o();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &tx).expect("serialization failed");
	let dtx: Transaction = ser::deserialize_default(&mut &vec[..]).unwrap();
	assert_eq!(dtx.fee(), 2);
	assert_eq!(dtx.inputs().len(), 2);
	assert_eq!(dtx.outputs().len(), 1);
	assert_eq!(tx.hash(), dtx.hash());
}

#[test]
fn tx_double_ser_deser() {
	test_setup();
	// checks serializing doesn't mess up the tx and produces consistent results
	let btx = tx2i1o();

	let mut vec = Vec::new();
	assert!(ser::serialize_default(&mut vec, &btx).is_ok());
	let dtx: Transaction = ser::deserialize_default(&mut &vec[..]).unwrap();

	let mut vec2 = Vec::new();
	assert!(ser::serialize_default(&mut vec2, &btx).is_ok());
	let dtx2: Transaction = ser::deserialize_default(&mut &vec2[..]).unwrap();

	assert_eq!(btx.hash(), dtx.hash());
	assert_eq!(dtx.hash(), dtx2.hash());
}

#[test]
fn test_zero_commit_fails() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	// blinding should fail as signing with a zero r*G shouldn't work
	let res = build::transaction(
		KernelFeatures::Plain {
			fee: FeeFields::zero(),
		},
		&[input(10, key_id1.clone()), output(10, key_id1)],
		&keychain,
		&builder,
	);
	assert!(res.is_err());
}

#[test]
fn build_tx_kernel() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	// first build a valid tx with corresponding blinding factor
	let tx = build::transaction(
		KernelFeatures::Plain { fee: 2.into() },
		&[input(10, key_id1), output(5, key_id2), output(3, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap();

	// check the tx is valid
	tx.validate(Weighting::AsTransaction).unwrap();

	// check the kernel is also itself valid
	assert_eq!(tx.kernels().len(), 1);
	let kern = &tx.kernels()[0];
	kern.verify().unwrap();

	assert_eq!(kern.features, KernelFeatures::Plain { fee: 2.into() });
	assert_eq!(2, tx.fee());
}

// Proof of concept demonstrating we can build two transactions that share
// the *same* kernel public excess. This is a key part of building a transaction as two
// "halves" for NRD kernels.
// Note: In a real world scenario multiple participants would build the kernel signature
// using signature aggregation. No party would see the full private kernel excess and
// the halves would need to be constructed with carefully crafted individual offsets to
// adjust the excess as required.
// For the sake of convenience we are simply constructing the kernel directly and we have access
// to the full private excess.
#[test]
fn build_two_half_kernels() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	// build kernel with associated private excess
	let mut kernel = TxKernel::with_features(KernelFeatures::Plain { fee: 2.into() });

	// Construct the message to be signed.
	let msg = kernel.msg_to_sign().unwrap();

	// Generate a kernel with public excess and associated signature.
	let excess = BlindingFactor::rand(&keychain.secp());
	let skey = excess.secret_key(&keychain.secp()).unwrap();
	kernel.excess = keychain.secp().commit(0, skey).unwrap();
	let pubkey = &kernel.excess.to_pubkey(&keychain.secp()).unwrap();
	kernel.excess_sig =
		aggsig::sign_with_blinding(&keychain.secp(), &msg, &excess, Some(&pubkey)).unwrap();
	kernel.verify().unwrap();

	let tx1 = build::transaction_with_kernel(
		&[input(10, key_id1), output(8, key_id2.clone())],
		kernel.clone(),
		excess.clone(),
		&keychain,
		&builder,
	)
	.unwrap();

	let tx2 = build::transaction_with_kernel(
		&[input(8, key_id2), output(6, key_id3)],
		kernel.clone(),
		excess.clone(),
		&keychain,
		&builder,
	)
	.unwrap();

	assert_eq!(tx1.validate(Weighting::AsTransaction), Ok(()),);

	assert_eq!(tx2.validate(Weighting::AsTransaction), Ok(()),);

	// The transactions share an identical kernel.
	assert_eq!(tx1.kernels()[0], tx2.kernels()[0]);

	// The public kernel excess is shared between both "halves".
	assert_eq!(tx1.kernels()[0].excess(), tx2.kernels()[0].excess());

	// Each transaction is built from different inputs and outputs.
	// The offset differs to compensate for the shared excess commitments.
	assert!(tx1.offset != tx2.offset);

	// For completeness, these are different transactions.
	assert!(tx1.hash() != tx2.hash());
}

// Combine two transactions into one big transaction (with multiple kernels)
// and check it still validates.
#[test]
fn transaction_cut_through() {
	test_setup();
	let tx1 = tx1i2o();
	let tx2 = tx2i1o();


	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());

	// now build a "cut_through" tx from tx1 and tx2
	let tx3 = aggregate(&[tx1, tx2]).unwrap();

	assert!(tx3.validate(Weighting::AsTransaction).is_ok());
}

// Attempt to deaggregate a multi-kernel transaction in a different way
#[test]
fn multi_kernel_transaction_deaggregation() {
	test_setup();
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();
	let tx4 = tx1i1o();

	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());
	assert!(tx3.validate(Weighting::AsTransaction).is_ok());
	assert!(tx4.validate(Weighting::AsTransaction).is_ok());

	let tx1234 = aggregate(&[tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()]).unwrap();
	let tx12 = aggregate(&[tx1, tx2]).unwrap();
	let tx34 = aggregate(&[tx3, tx4]).unwrap();

	assert!(tx1234.validate(Weighting::AsTransaction).is_ok());
	assert!(tx12.validate(Weighting::AsTransaction).is_ok());
	assert!(tx34.validate(Weighting::AsTransaction).is_ok());

	let deaggregated_tx34 = deaggregate(tx1234.clone(), &[tx12.clone()]).unwrap();
	assert!(deaggregated_tx34
		.validate(Weighting::AsTransaction)

		.is_ok());
	assert_eq!(tx34, deaggregated_tx34);

	let deaggregated_tx12 = deaggregate(tx1234, &[tx34]).unwrap();

	assert!(deaggregated_tx12
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx12, deaggregated_tx12);
}

#[test]
fn multi_kernel_transaction_deaggregation_2() {
	test_setup();
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();

	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());
	assert!(tx3.validate(Weighting::AsTransaction).is_ok());

	let tx123 = aggregate(&[tx1.clone(), tx2.clone(), tx3.clone()]).unwrap();
	let tx12 = aggregate(&[tx1, tx2]).unwrap();

	assert!(tx123.validate(Weighting::AsTransaction).is_ok());
	assert!(tx12.validate(Weighting::AsTransaction).is_ok());

	let deaggregated_tx3 = deaggregate(tx123, &[tx12]).unwrap();
	assert!(deaggregated_tx3
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx3, deaggregated_tx3);
}

#[test]
fn multi_kernel_transaction_deaggregation_3() {
	test_setup();
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();

	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());
	assert!(tx3.validate(Weighting::AsTransaction).is_ok());

	let tx123 = aggregate(&[tx1.clone(), tx2.clone(), tx3.clone()]).unwrap();
	let tx13 = aggregate(&[tx1, tx3]).unwrap();
	let tx2 = aggregate(&[tx2]).unwrap();

	assert!(tx123.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());

	let deaggregated_tx13 = deaggregate(tx123, &[tx2]).unwrap();
	assert!(deaggregated_tx13
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx13, deaggregated_tx13);
}

#[test]
fn multi_kernel_transaction_deaggregation_4() {
	test_setup();
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();
	let tx4 = tx1i1o();
	let tx5 = tx1i1o();

	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());
	assert!(tx3.validate(Weighting::AsTransaction).is_ok());
	assert!(tx4.validate(Weighting::AsTransaction).is_ok());
	assert!(tx5.validate(Weighting::AsTransaction).is_ok());

	let tx12345 = aggregate(&[
		tx1.clone(),
		tx2.clone(),
		tx3.clone(),
		tx4.clone(),
		tx5.clone(),
	])
	.unwrap();
	assert!(tx12345.validate(Weighting::AsTransaction).is_ok());

	let deaggregated_tx5 = deaggregate(tx12345, &[tx1, tx2, tx3, tx4]).unwrap();
	assert!(deaggregated_tx5
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx5, deaggregated_tx5);
}

#[test]
fn multi_kernel_transaction_deaggregation_5() {
	test_setup();
	let tx1 = tx1i1o();
	let tx2 = tx1i1o();
	let tx3 = tx1i1o();
	let tx4 = tx1i1o();
	let tx5 = tx1i1o();

	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());
	assert!(tx3.validate(Weighting::AsTransaction).is_ok());
	assert!(tx4.validate(Weighting::AsTransaction).is_ok());
	assert!(tx5.validate(Weighting::AsTransaction).is_ok());

	let tx12345 = aggregate(&[
		tx1.clone(),
		tx2.clone(),
		tx3.clone(),
		tx4.clone(),
		tx5.clone(),
	])
	.unwrap();
	let tx12 = aggregate(&[tx1, tx2]).unwrap();
	let tx34 = aggregate(&[tx3, tx4]).unwrap();

	assert!(tx12345.validate(Weighting::AsTransaction).is_ok());

	let deaggregated_tx5 = deaggregate(tx12345, &[tx12, tx34]).unwrap();
	assert!(deaggregated_tx5
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx5, deaggregated_tx5);
}

// Attempt to deaggregate a multi-kernel transaction
#[test]
fn basic_transaction_deaggregation() {
	test_setup();
	let tx1 = tx1i2o();
	let tx2 = tx2i1o();

	assert!(tx1.validate(Weighting::AsTransaction).is_ok());
	assert!(tx2.validate(Weighting::AsTransaction).is_ok());

	// now build a "cut_through" tx from tx1 and tx2
	let tx3 = aggregate(&[tx1.clone(), tx2.clone()]).unwrap();

	assert!(tx3.validate(Weighting::AsTransaction).is_ok());

	let deaggregated_tx1 = deaggregate(tx3.clone(), &[tx2.clone()]).unwrap();

	assert!(deaggregated_tx1
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx1, deaggregated_tx1);

	let deaggregated_tx2 = deaggregate(tx3, &[tx1]).unwrap();

	assert!(deaggregated_tx2
		.validate(Weighting::AsTransaction)
		.is_ok());
	assert_eq!(tx2, deaggregated_tx2);
}

#[test]
fn hash_output() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx = build::transaction(
		KernelFeatures::Plain { fee: 1.into() },
		&[input(75, key_id1), output(42, key_id2), output(32, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap();
	let h = tx.outputs()[0].identifier.hash();
	assert!(h != ZERO_HASH);
	let h2 = tx.outputs()[1].identifier.hash();
	assert!(h != h2);
}

#[ignore]
#[test]
fn blind_tx() {
	let btx = tx2i1o();
	assert!(btx.validate(Weighting::AsTransaction).is_ok());

	// Ignored for bullet proofs, because calling range_proof_info
	// with a bullet proof causes painful errors

	// checks that the range proof on our blind output is sufficiently hiding
	let Output { proof, .. } = btx.outputs()[0];

	let secp = static_secp_instance();
	let secp = secp.lock();
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
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
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
		let tx = Transaction::empty().with_kernel(TxKernel::with_features(KernelFeatures::Plain {
			fee: 2.into(),
		}));
		let (tx, sum) =
			build::partial_transaction(tx, &[in1, in2, output(1, key_id3)], &keychain, &builder)
				.unwrap();

		(tx, sum)
	};

	// From now on, Bob only has the obscured transaction and the sum of
	// blinding factors. He adds his output, finalizes the transaction so it's
	// ready for broadcast.
	let tx_final = build::transaction(
		KernelFeatures::Plain { fee: 2.into() },
		&[
			initial_tx(tx_alice),
			with_excess(blind_sum),
			output(4, key_id4),
		],
		&keychain,
		&builder,
	)
	.unwrap();

	tx_final.validate(Weighting::AsTransaction).unwrap();
}

#[test]
fn reward_empty_block() {
	test_setup();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let previous_header = BlockHeader::default();

	let b = new_block(&[], &keychain, &builder, &previous_header, &key_id);

	b.validate(&BlindingFactor::zero()).unwrap();
}

#[test]
fn reward_with_tx_block() {
	test_setup();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let tx1 = tx2i1o();
	let previous_header = BlockHeader::default();
	tx1.validate(Weighting::AsTransaction)
		.unwrap();

	let block = new_block(&[tx1], &keychain, &builder, &previous_header, &key_id);
	block.validate(&BlindingFactor::zero()).unwrap();
}

#[test]
fn simple_block() {
	test_setup();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let tx1 = tx2i1o();
	let tx2 = tx1i1o();

	let previous_header = BlockHeader::default();
	let b = new_block(&[tx1, tx2], &keychain, &builder, &previous_header, &key_id);

	b.validate(&BlindingFactor::zero()).unwrap();
}

#[test]
fn test_block_with_timelocked_tx() {
	test_setup();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	// first check we can add a timelocked tx where lock height matches current
	// block height and that the resulting block is valid
	let tx1 = build::transaction(
		KernelFeatures::HeightLocked {
			fee: 2.into(),
			lock_height: 1,
		},
		&[input(5, key_id1.clone()), output(3, key_id2.clone())],
		&keychain,
		&builder,
	)
	.unwrap();

	let previous_header = BlockHeader::default();

	let b = new_block(
		&[tx1],
		&keychain,
		&builder,
		&previous_header,
		&key_id3.clone(),
	);
	b.validate(&BlindingFactor::zero()).unwrap();

	// now try adding a timelocked tx where lock height is greater than current
	// block height
	let tx1 = build::transaction(
		KernelFeatures::HeightLocked {
			fee: 2.into(),
			lock_height: 2,
		},
		&[input(5, key_id1), output(3, key_id2)],
		&keychain,
		&builder,
	)
	.unwrap();

	let previous_header = BlockHeader::default();
	let b = new_block(&[tx1], &keychain, &builder, &previous_header, &key_id3);

	match b.validate(&BlindingFactor::zero()) {
		Err(KernelLockHeight(height)) => {
			assert_eq!(height, 2);
		}
		_ => panic!("expecting KernelLockHeight error here"),
	}
}

#[test]
pub fn test_verify_1i1o_sig() {
	test_setup();
	let tx = tx1i1o();
	tx.validate(Weighting::AsTransaction).unwrap();
}

#[test]
pub fn test_verify_2i1o_sig() {
	test_setup();
	let tx = tx2i1o();
	tx.validate(Weighting::AsTransaction).unwrap();
}
