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

//! Transaction integration tests

pub mod common;
use crate::common::tx1i10_v2_compatible;
use crate::core::core::transaction::{self, Error};
use crate::core::core::{
	FeeFields, KernelFeatures, Output, OutputFeatures, Transaction, TxKernel, Weighting,
};
use crate::core::global;
use crate::core::libtx::proof::{self, ProofBuilder};
use crate::core::libtx::{build, tx_fee};
use crate::core::{consensus, ser};
use grin_core as core;
use keychain::{ExtKeychain, Keychain};

// We use json serialization between wallet->node when pushing transactions to the network.
// This test ensures we exercise this serialization/deserialization code.
#[test]
fn test_transaction_json_ser_deser() {
	let tx1 = tx1i10_v2_compatible();

	let value = serde_json::to_value(&tx1).unwrap();
	println!("{:?}", value);

	assert!(value["offset"].is_string());
	assert_eq!(value["body"]["inputs"][0]["features"], "Plain");
	assert!(value["body"]["inputs"][0]["commit"].is_string());
	assert_eq!(value["body"]["outputs"][0]["features"], "Plain");
	assert!(value["body"]["outputs"][0]["commit"].is_string());
	assert!(value["body"]["outputs"][0]["proof"].is_string());

	// Note: Tx kernel "features" serialize in a slightly unexpected way.
	assert_eq!(value["body"]["kernels"][0]["features"]["Plain"]["fee"], 2);
	assert!(value["body"]["kernels"][0]["excess"].is_string());
	assert!(value["body"]["kernels"][0]["excess_sig"].is_string());

	let tx2: Transaction = serde_json::from_value(value).unwrap();
	assert_eq!(tx1, tx2);

	let str = serde_json::to_string(&tx1).unwrap();
	println!("{}", str);
	let tx2: Transaction = serde_json::from_str(&str).unwrap();
	assert_eq!(tx1, tx2);
}

#[test]
fn test_output_ser_deser() {
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let switch = keychain::SwitchCommitmentType::Regular;
	let commit = keychain.commit(5, &key_id, switch).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let proof = proof::create(&keychain, &builder, 5, &key_id, switch, commit, None).unwrap();

	let out = Output::new(OutputFeatures::Plain, commit, proof);

	let mut vec = vec![];
	ser::serialize_default(&mut vec, &out).expect("serialized failed");
	let dout: Output = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(dout.features(), OutputFeatures::Plain);
	assert_eq!(dout.commitment(), out.commitment());
	assert_eq!(dout.proof, out.proof);
}

// Test coverage for verifying cut-through during transaction validation.
// It is not valid for a transaction to spend an output and produce a new output with the same commitment.
// This test covers the case where a plain output is spent, producing a plain output with the same commitment.
#[test]
fn test_verify_cut_through_plain() -> Result<(), Error> {
	global::set_local_chain_type(global::ChainTypes::UserTesting);

	let keychain = ExtKeychain::from_random_seed(false)?;

	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let builder = proof::ProofBuilder::new(&keychain);

	let mut tx = build::transaction(
		KernelFeatures::Plain {
			fee: FeeFields::zero(),
		},
		&[
			build::input(10, key_id1.clone()),
			build::input(10, key_id2.clone()),
			build::output(10, key_id1.clone()),
			build::output(6, key_id2.clone()),
			build::output(4, key_id3.clone()),
		],
		&keychain,
		&builder,
	)
	.expect("valid tx");

	// Transaction should fail validation due to cut-through.
	assert_eq!(
		tx.validate(Weighting::AsTransaction),
		Err(Error::CutThrough),
	);

	// Transaction should fail lightweight "read" validation due to cut-through.
	assert_eq!(tx.validate_read(), Err(Error::CutThrough));

	// Apply cut-through to eliminate the offending input and output.
	let mut inputs: Vec<_> = tx.inputs().into();
	let mut outputs = tx.outputs().to_vec();
	let (inputs, outputs, _, _) = transaction::cut_through(&mut inputs[..], &mut outputs[..])?;

	tx.body = tx
		.body
		.replace_inputs(inputs.into())
		.replace_outputs(outputs);

	// Transaction validates successfully after applying cut-through.
	tx.validate(Weighting::AsTransaction)?;

	// Transaction validates via lightweight "read" validation as well.
	tx.validate_read()?;

	Ok(())
}

// Test coverage for verifying cut-through during transaction validation.
// It is not valid for a transaction to spend an output and produce a new output with the same commitment.
// This test covers the case where a coinbase output is spent, producing a plain output with the same commitment.
#[test]
fn test_verify_cut_through_coinbase() -> Result<(), Error> {
	global::set_local_chain_type(global::ChainTypes::UserTesting);

	let keychain = ExtKeychain::from_random_seed(false)?;

	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let builder = ProofBuilder::new(&keychain);

	let mut tx = build::transaction(
		KernelFeatures::Plain {
			fee: FeeFields::zero(),
		},
		&[
			build::coinbase_input(consensus::REWARD, key_id1.clone()),
			build::coinbase_input(consensus::REWARD, key_id2.clone()),
			build::output(60_000_000_000, key_id1.clone()),
			build::output(50_000_000_000, key_id2.clone()),
			build::output(10_000_000_000, key_id3.clone()),
		],
		&keychain,
		&builder,
	)
	.expect("valid tx");

	// Transaction should fail validation due to cut-through.
	assert_eq!(
		tx.validate(Weighting::AsTransaction),
		Err(Error::CutThrough),
	);

	// Transaction should fail lightweight "read" validation due to cut-through.
	assert_eq!(tx.validate_read(), Err(Error::CutThrough));

	// Apply cut-through to eliminate the offending input and output.
	let mut inputs: Vec<_> = tx.inputs().into();
	let mut outputs = tx.outputs().to_vec();
	let (inputs, outputs, _, _) = transaction::cut_through(&mut inputs[..], &mut outputs[..])?;

	tx.body = tx
		.body
		.replace_inputs(inputs.into())
		.replace_outputs(outputs);

	// Transaction validates successfully after applying cut-through.
	tx.validate(Weighting::AsTransaction)?;

	// Transaction validates via lightweight "read" validation as well.
	tx.validate_read()?;

	Ok(())
}

// Test coverage for FeeFields
#[test]
fn test_fee_fields() -> Result<(), Error> {
	global::set_local_chain_type(global::ChainTypes::UserTesting);
	global::set_local_accept_fee_base(500_000);

	let keychain = ExtKeychain::from_random_seed(false)?;

	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);

	let builder = ProofBuilder::new(&keychain);

	let mut tx = build::transaction(
		KernelFeatures::Plain {
			fee: FeeFields::new(1, 42).unwrap(),
		},
		&[
			build::coinbase_input(consensus::REWARD, key_id1.clone()),
			build::output(60_000_000_000 - 84 - 42 - 21, key_id1.clone()),
		],
		&keychain,
		&builder,
	)
	.expect("valid tx");

	assert_eq!(tx.accept_fee(), (1 * 1 + 1 * 21 + 1 * 3) * 500_000);
	assert_eq!(tx.fee(), 42);
	assert_eq!(tx.shifted_fee(), 21);

	tx.body.kernels.append(&mut vec![
		TxKernel::with_features(KernelFeatures::Plain {
			fee: FeeFields::new(2, 84).unwrap(),
		}),
		TxKernel::with_features(KernelFeatures::Plain { fee: 21.into() }),
	]);

	assert_eq!(tx.fee(), 147);
	assert_eq!(tx.shifted_fee(), 36);
	assert_eq!(tx.aggregate_fee_fields(), FeeFields::new(2, 147));
	assert_eq!(tx_fee(1, 1, 3), 15_500_000);

	Ok(())
}
