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

mod common;
use crate::common::{new_block, tx1i2o, tx2i1o, txspend1i1o};
use crate::core::consensus::{self, OUTPUT_WEIGHT, TESTING_HARD_FORK_INTERVAL};
use crate::core::core::block::{Block, BlockHeader, Error, HeaderVersion, UntrustedBlockHeader};
use crate::core::core::hash::Hashed;
use crate::core::core::id::ShortIdentifiable;
use crate::core::core::transaction::{
	self, FeeFields, KernelFeatures, NRDRelativeHeight, Output, OutputFeatures, OutputIdentifier,
	Transaction,
};
use crate::core::core::{Committed, CompactBlock};
use crate::core::libtx::build::{self, input, output};
use crate::core::libtx::ProofBuilder;
use crate::core::{global, pow, ser};
use chrono::Duration;
use grin_core as core;
use keychain::{BlindingFactor, ExtKeychain, Keychain};
use util::{secp, ToHex};

// Setup test with AutomatedTesting chain_type;
fn test_setup() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
}

#[test]
fn too_large_block() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let max_out = global::max_block_weight() / OUTPUT_WEIGHT;

	let mut pks = vec![];
	for n in 0..(max_out + 1) {
		pks.push(ExtKeychain::derive_key_id(1, n as u32, 0, 0, 0));
	}

	let mut parts = vec![];
	for _ in 0..max_out {
		parts.push(output(5, pks.pop().unwrap()));
	}

	parts.append(&mut vec![input(500000, pks.pop().unwrap())]);
	let tx = build::transaction(
		KernelFeatures::Plain { fee: 2.into() },
		&parts,
		&keychain,
		&builder,
	)
	.unwrap();

	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx], &keychain, &builder, &prev, &key_id);
	assert!(b.validate(&BlindingFactor::zero()).is_err());
}

#[test]
// block with no inputs/outputs/kernels
// no fees, no reward, no coinbase
fn very_empty_block() {
	test_setup();
	let b = Block::with_header(BlockHeader::default());

	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);
}

#[test]
fn block_with_nrd_kernel_pre_post_hf3() {
	// automated testing - HF{1|2|3} at block heights {3, 6, 9}
	// Enable the global NRD feature flag. NRD kernels valid at HF3 at height 9.
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_nrd_enabled(true);
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);

	let tx = build::transaction(
		KernelFeatures::NoRecentDuplicate {
			fee: 2.into(),
			relative_height: NRDRelativeHeight::new(1440).unwrap(),
		},
		&[input(7, key_id1), output(5, key_id2)],
		&keychain,
		&builder,
	)
	.unwrap();
	let txs = &[tx];

	let prev_height = 3 * TESTING_HARD_FORK_INTERVAL - 2;
	let prev = BlockHeader {
		height: prev_height,
		version: consensus::header_version(prev_height),
		..BlockHeader::default()
	};
	let b = new_block(
		txs,
		&keychain,
		&builder,
		&prev,
		&ExtKeychain::derive_key_id(1, 1, 0, 0, 0),
	);

	// Block is invalid at header version 3 if it contains an NRD kernel.
	assert_eq!(b.header.version, HeaderVersion(3));
	assert_eq!(
		b.validate(&BlindingFactor::zero()),
		Err(Error::NRDKernelPreHF3)
	);

	let prev_height = 3 * TESTING_HARD_FORK_INTERVAL - 1;
	let prev = BlockHeader {
		height: prev_height,
		version: consensus::header_version(prev_height),
		..BlockHeader::default()
	};
	let b = new_block(
		txs,
		&keychain,
		&builder,
		&prev,
		&ExtKeychain::derive_key_id(1, 1, 0, 0, 0),
	);

	// Block is valid at header version 4 (at HF height) if it contains an NRD kernel.
	assert_eq!(b.header.height, 3 * TESTING_HARD_FORK_INTERVAL);
	assert_eq!(b.header.version, HeaderVersion(4));
	assert!(b.validate(&BlindingFactor::zero()).is_ok());

	let prev_height = 3 * TESTING_HARD_FORK_INTERVAL;
	let prev = BlockHeader {
		height: prev_height,
		version: consensus::header_version(prev_height),
		..BlockHeader::default()
	};
	let b = new_block(
		txs,
		&keychain,
		&builder,
		&prev,
		&ExtKeychain::derive_key_id(1, 1, 0, 0, 0),
	);

	// Block is valid at header version 4 if it contains an NRD kernel.
	assert_eq!(b.header.version, HeaderVersion(4));
	assert!(b.validate(&BlindingFactor::zero()).is_ok());
}

#[test]
fn block_with_nrd_kernel_nrd_not_enabled() {
	// automated testing - HF{1|2|3} at block heights {3, 6, 9}
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);

	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);

	let tx = build::transaction(
		KernelFeatures::NoRecentDuplicate {
			fee: 2.into(),
			relative_height: NRDRelativeHeight::new(1440).unwrap(),
		},
		&[input(7, key_id1), output(5, key_id2)],
		&keychain,
		&builder,
	)
	.unwrap();

	let txs = &[tx];

	let prev_height = 3 * TESTING_HARD_FORK_INTERVAL - 2;
	let prev = BlockHeader {
		height: prev_height,
		version: consensus::header_version(prev_height),
		..BlockHeader::default()
	};
	let b = new_block(
		txs,
		&keychain,
		&builder,
		&prev,
		&ExtKeychain::derive_key_id(1, 1, 0, 0, 0),
	);

	// Block is invalid as NRD not enabled.
	assert_eq!(b.header.version, HeaderVersion(3));
	assert_eq!(
		b.validate(&BlindingFactor::zero()),
		Err(Error::NRDKernelNotEnabled)
	);

	let prev_height = 3 * TESTING_HARD_FORK_INTERVAL - 1;
	let prev = BlockHeader {
		height: prev_height,
		version: consensus::header_version(prev_height),
		..BlockHeader::default()
	};
	let b = new_block(
		txs,
		&keychain,
		&builder,
		&prev,
		&ExtKeychain::derive_key_id(1, 1, 0, 0, 0),
	);

	// Block is invalid as NRD not enabled.
	assert_eq!(b.header.height, 3 * TESTING_HARD_FORK_INTERVAL);
	assert_eq!(b.header.version, HeaderVersion(4));
	assert_eq!(
		b.validate(&BlindingFactor::zero()),
		Err(Error::NRDKernelNotEnabled)
	);

	let prev_height = 3 * TESTING_HARD_FORK_INTERVAL;
	let prev = BlockHeader {
		height: prev_height,
		version: consensus::header_version(prev_height),
		..BlockHeader::default()
	};
	let b = new_block(
		txs,
		&keychain,
		&builder,
		&prev,
		&ExtKeychain::derive_key_id(1, 1, 0, 0, 0),
	);

	// Block is invalid as NRD not enabled.
	assert_eq!(b.header.version, HeaderVersion(4));
	assert_eq!(
		b.validate(&BlindingFactor::zero()),
		Err(Error::NRDKernelNotEnabled)
	);
}

#[test]
// builds a block with a tx spending another and check that cut_through occurred
fn block_with_cut_through() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let btx1 = tx2i1o();
	let btx2 = build::transaction(
		KernelFeatures::Plain { fee: 2.into() },
		&[input(7, key_id1), output(5, key_id2.clone())],
		&keychain,
		&builder,
	)
	.unwrap();

	// spending tx2 - reuse key_id2

	let btx3 = txspend1i1o(5, &keychain, &builder, key_id2, key_id3);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[btx1, btx2, btx3], &keychain, &builder, &prev, &key_id);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	b.validate(&BlindingFactor::zero()).unwrap();
	assert_eq!(b.inputs().len(), 3);
	assert_eq!(b.outputs().len(), 3);
}

#[test]
fn empty_block_with_coinbase_is_valid() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);

	assert_eq!(b.inputs().len(), 0);
	assert_eq!(b.outputs().len(), 1);
	assert_eq!(b.kernels().len(), 1);

	let coinbase_outputs = b
		.outputs()
		.iter()
		.filter(|out| out.is_coinbase())
		.cloned()
		.collect::<Vec<_>>();
	assert_eq!(coinbase_outputs.len(), 1);

	let coinbase_kernels = b
		.kernels()
		.iter()
		.filter(|out| out.is_coinbase())
		.cloned()
		.collect::<Vec<_>>();
	assert_eq!(coinbase_kernels.len(), 1);

	// the block should be valid here (single coinbase output with corresponding
	// txn kernel)
	assert!(b.validate(&BlindingFactor::zero()).is_ok());
}

#[test]
// test that flipping the COINBASE flag on the output features
// invalidates the block and specifically it causes verify_coinbase to fail
// additionally verifying the merkle_inputs_outputs also fails
fn remove_coinbase_output_flag() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);
	let output = b.outputs()[0];
	let output = Output::new(OutputFeatures::Plain, output.commitment(), output.proof());
	let b = Block {
		body: b.body.replace_outputs(&[output]),
		..b
	};

	assert_eq!(b.verify_coinbase(), Err(Error::CoinbaseSumMismatch));
	assert!(b
		.verify_kernel_sums(b.header.overage(), b.header.total_kernel_offset())
		.is_ok());
	assert_eq!(
		b.validate(&BlindingFactor::zero()),
		Err(Error::CoinbaseSumMismatch)
	);
}

#[test]
// test that flipping the COINBASE flag on the kernel features
// invalidates the block and specifically it causes verify_coinbase to fail
fn remove_coinbase_kernel_flag() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let mut b = new_block(&[], &keychain, &builder, &prev, &key_id);

	let mut kernel = b.kernels()[0].clone();
	kernel.features = KernelFeatures::Plain {
		fee: FeeFields::zero(),
	};
	b.body = b.body.replace_kernel(kernel);

	// Flipping the coinbase flag results in kernels not summing correctly.
	assert_eq!(
		b.verify_coinbase(),
		Err(Error::Secp(secp::Error::IncorrectCommitSum))
	);

	// Also results in the block no longer validating correctly
	// because the message being signed on each tx kernel includes the kernel features.
	assert_eq!(
		b.validate(&BlindingFactor::zero()),
		Err(Error::Transaction(transaction::Error::IncorrectSignature))
	);
}

#[test]
fn serialize_deserialize_header_version() {
	let mut vec1 = Vec::new();
	ser::serialize_default(&mut vec1, &1_u16).expect("serialization failed");

	let mut vec2 = Vec::new();
	ser::serialize_default(&mut vec2, &HeaderVersion(1)).expect("serialization failed");

	// Check that a header_version serializes to a
	// single u16 value with no extraneous bytes wrapping it.
	assert_eq!(vec1, vec2);

	// Check we can successfully deserialize a header_version.
	let version: HeaderVersion = ser::deserialize_default(&mut &vec2[..]).unwrap();
	assert_eq!(version.0, 1)
}

#[test]
fn serialize_deserialize_block_header() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);
	let header1 = b.header;

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &header1).expect("serialization failed");
	let header2: BlockHeader = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(header1.hash(), header2.hash());
	assert_eq!(header1, header2);
}

fn set_pow(header: &mut BlockHeader) {
	// Set valid pow on the block as we will test deserialization of this "untrusted" from the network.
	let edge_bits = global::min_edge_bits();
	header.pow.proof.edge_bits = edge_bits;
	pow::pow_size(
		header,
		pow::Difficulty::min_dma(),
		global::proofsize(),
		edge_bits,
	)
	.unwrap();
}

#[test]
fn deserialize_untrusted_header_weight() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let mut b = new_block(&[], &keychain, &builder, &prev, &key_id);

	// Set excessively large output mmr size on the header.
	b.header.output_mmr_size = 10_000;
	b.header.kernel_mmr_size = 0;
	set_pow(&mut b.header);

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b.header).expect("serialization failed");
	let res: Result<UntrustedBlockHeader, _> = ser::deserialize_default(&mut &vec[..]);
	assert_eq!(res.err(), Some(ser::Error::CorruptedData));

	// Set excessively large kernel mmr size on the header.
	b.header.output_mmr_size = 0;
	b.header.kernel_mmr_size = 10_000;
	set_pow(&mut b.header);

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b.header).expect("serialization failed");
	let res: Result<UntrustedBlockHeader, _> = ser::deserialize_default(&mut &vec[..]);
	assert_eq!(res.err(), Some(ser::Error::CorruptedData));

	// Set reasonable mmr sizes on the header to confirm the header can now be read "untrusted".
	b.header.output_mmr_size = 1;
	b.header.kernel_mmr_size = 1;
	set_pow(&mut b.header);

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b.header).expect("serialization failed");
	let res: Result<UntrustedBlockHeader, _> = ser::deserialize_default(&mut &vec[..]);
	assert!(res.is_ok());
}

#[test]
fn serialize_deserialize_block() {
	test_setup();
	let tx1 = tx1i2o();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx1], &keychain, &builder, &prev, &key_id);

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	let b2: Block = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(b.hash(), b2.hash());
	assert_eq!(b.header, b2.header);
	assert_eq!(b.inputs(), b2.inputs());
	assert_eq!(b.outputs(), b2.outputs());
	assert_eq!(b.kernels(), b2.kernels());
}

#[test]
fn empty_block_serialized_size() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	assert_eq!(vec.len(), 1_096);
}

#[test]
fn block_single_tx_serialized_size() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx1], &keychain, &builder, &prev, &key_id);

	// Default protocol version (3)
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	assert_eq!(vec.len(), 2_669);

	// Protocol version 3
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(3), &b).expect("serialization failed");
	assert_eq!(vec.len(), 2_669);

	// Protocol version 2.
	// Note: block must be in "v2" compatibility with "features and commit" inputs for this.
	// Normally we would convert the block by looking inputs up in utxo but we fake it here for testing.
	let inputs: Vec<_> = b.inputs().into();
	let inputs: Vec<_> = inputs
		.iter()
		.map(|input| OutputIdentifier {
			features: OutputFeatures::Plain,
			commit: input.commitment(),
		})
		.collect();
	let b = Block {
		header: b.header,
		body: b.body.replace_inputs(inputs.as_slice().into()),
	};

	// Protocol version 2
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(2), &b).expect("serialization failed");
	assert_eq!(vec.len(), 2_670);

	// Protocol version 1 (fixed size kernels)
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(1), &b).expect("serialization failed");
	assert_eq!(vec.len(), 2_694);

	// Check we can also serialize a v2 compatibility block in v3 protocol version
	// without needing to explicitly convert the block.
	let mut vec = Vec::new();
	ser::serialize(&mut vec, ser::ProtocolVersion(3), &b).expect("serialization failed");
	assert_eq!(vec.len(), 2_669);

	// Default protocol version (3) for completeness
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &b).expect("serialization failed");
	assert_eq!(vec.len(), 2_669);
}

#[test]
fn empty_compact_block_serialized_size() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb).expect("serialization failed");
	assert_eq!(vec.len(), 1_104);
}

#[test]
fn compact_block_single_tx_serialized_size() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx1], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb).expect("serialization failed");
	assert_eq!(vec.len(), 1_110);
}

#[test]
fn block_10_tx_serialized_size() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&txs, &keychain, &builder, &prev, &key_id);

	{
		let mut vec = Vec::new();
		ser::serialize_default(&mut vec, &b).expect("serialization failed");
		assert_eq!(vec.len(), 16_826);
	}
}

#[test]
fn compact_block_10_tx_serialized_size() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);

	let mut txs = vec![];
	for _ in 0..10 {
		let tx = tx1i2o();
		txs.push(tx);
	}
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&txs, &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.into();
	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb).expect("serialization failed");
	assert_eq!(vec.len(), 1_164);
}

#[test]
fn compact_block_hash_with_nonce() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx.clone()], &keychain, &builder, &prev, &key_id);
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
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx1], &keychain, &builder, &prev, &key_id);
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
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);
	let cb: CompactBlock = b.clone().into();
	let hb = Block::hydrate_from(cb, &[]).unwrap();
	assert_eq!(hb.header, b.header);
	assert_eq!(hb.outputs(), b.outputs());
	assert_eq!(hb.kernels(), b.kernels());
}

#[test]
fn serialize_deserialize_compact_block() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let tx1 = tx1i2o();
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[tx1], &keychain, &builder, &prev, &key_id);

	let mut cb1: CompactBlock = b.into();

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &cb1).expect("serialization failed");

	// After header serialization, timestamp will lose 'nanos' info, that's the designed behavior.
	// To suppress 'nanos' difference caused assertion fail, we force b.header also lose 'nanos'.
	let origin_ts = cb1.header.timestamp;
	cb1.header.timestamp =
		origin_ts - Duration::nanoseconds(origin_ts.timestamp_subsec_nanos() as i64);

	let cb2: CompactBlock = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(cb1.header, cb2.header);
	assert_eq!(cb1.kern_ids(), cb2.kern_ids());
}

// Duplicate a range proof from a valid output into another of the same amount
#[test]
fn same_amount_outputs_copy_range_proof() {
	test_setup();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx = build::transaction(
		KernelFeatures::Plain { fee: 1.into() },
		&[input(7, key_id1), output(3, key_id2), output(3, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap();

	// now we reconstruct the transaction, swapping the rangeproofs so they
	// have the wrong privkey
	let mut outs = tx.outputs().to_vec();
	outs[0].proof = outs[1].proof;

	let key_id = keychain::ExtKeychain::derive_key_id(1, 4, 0, 0, 0);
	let prev = BlockHeader::default();
	let b = new_block(
		&[Transaction::new(tx.inputs(), &outs, tx.kernels())],
		&keychain,
		&builder,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	match b.validate(&BlindingFactor::zero()) {
		Err(Error::Transaction(transaction::Error::Secp(secp::Error::InvalidRangeProof))) => {}
		_ => panic!("Bad range proof should be invalid"),
	}
}

// Swap a range proof with the right private key but wrong amount
#[test]
fn wrong_amount_range_proof() {
	test_setup();
	let keychain = keychain::ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let key_id1 = keychain::ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = keychain::ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = keychain::ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let tx1 = build::transaction(
		KernelFeatures::Plain { fee: 1.into() },
		&[
			input(7, key_id1.clone()),
			output(3, key_id2.clone()),
			output(3, key_id3.clone()),
		],
		&keychain,
		&builder,
	)
	.unwrap();
	let tx2 = build::transaction(
		KernelFeatures::Plain { fee: 1.into() },
		&[input(7, key_id1), output(2, key_id2), output(4, key_id3)],
		&keychain,
		&builder,
	)
	.unwrap();

	// we take the range proofs from tx2 into tx1 and rebuild the transaction
	let mut outs = tx1.outputs().to_vec();
	outs[0].proof = tx2.outputs()[0].proof;
	outs[1].proof = tx2.outputs()[1].proof;

	let key_id = keychain::ExtKeychain::derive_key_id(1, 4, 0, 0, 0);
	let prev = BlockHeader::default();
	let b = new_block(
		&[Transaction::new(tx1.inputs(), &outs, tx1.kernels())],
		&keychain,
		&builder,
		&prev,
		&key_id,
	);

	// block should have been automatically compacted (including reward
	// output) and should still be valid
	match b.validate(&BlindingFactor::zero()) {
		Err(Error::Transaction(transaction::Error::Secp(secp::Error::InvalidRangeProof))) => {}
		_ => panic!("Bad range proof should be invalid"),
	}
}

#[test]
fn validate_header_proof() {
	test_setup();
	let keychain = ExtKeychain::from_random_seed(false).unwrap();
	let builder = ProofBuilder::new(&keychain);
	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let b = new_block(&[], &keychain, &builder, &prev, &key_id);

	let mut header_buf = vec![];
	{
		let mut writer = ser::BinWriter::default(&mut header_buf);
		b.header.write_pre_pow(&mut writer).unwrap();
		b.header.pow.write_pre_pow(&mut writer).unwrap();
	}
	let pre_pow = header_buf.to_hex();

	let reconstructed = BlockHeader::from_pre_pow_and_proof(
		pre_pow,
		b.header.pow.nonce,
		b.header.pow.proof.clone(),
	)
	.unwrap();
	assert_eq!(reconstructed, b.header);

	// assert invalid pre_pow returns error
	assert!(BlockHeader::from_pre_pow_and_proof(
		"0xaf1678".to_string(),
		b.header.pow.nonce,
		b.header.pow.proof,
	)
	.is_err());
}

// Test coverage for verifying cut-through during block validation.
// It is not valid for a block to spend an output and produce a new output with the same commitment.
// This test covers the case where a plain output is spent, producing a plain output with the same commitment.
#[test]
fn test_verify_cut_through_plain() -> Result<(), Error> {
	global::set_local_chain_type(global::ChainTypes::UserTesting);

	let keychain = ExtKeychain::from_random_seed(false).unwrap();

	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let builder = ProofBuilder::new(&keychain);

	let tx = build::transaction(
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

	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(0, 0, 0, 0, 0);
	let mut block = new_block(&[tx], &keychain, &builder, &prev, &key_id);

	// The block should fail validation due to cut-through.
	assert_eq!(
		block.validate(&BlindingFactor::zero()),
		Err(Error::Transaction(transaction::Error::CutThrough))
	);

	// The block should fail lightweight "read" validation due to cut-through.
	assert_eq!(
		block.validate_read(),
		Err(Error::Transaction(transaction::Error::CutThrough))
	);

	// Apply cut-through to eliminate the offending input and output.
	let mut inputs: Vec<_> = block.inputs().into();
	let mut outputs = block.outputs().to_vec();
	let (inputs, outputs, _, _) = transaction::cut_through(&mut inputs[..], &mut outputs[..])?;

	block.body = block
		.body
		.replace_inputs(inputs.into())
		.replace_outputs(outputs);

	// Block validates successfully after applying cut-through.
	block.validate(&BlindingFactor::zero())?;

	// Block validates via lightweight "read" validation.
	block.validate_read()?;

	Ok(())
}

// Test coverage for verifying cut-through during block validation.
// It is not valid for a block to spend an output and produce a new output with the same commitment.
// This test covers the case where a coinbase output is spent, producing a plain output with the same commitment.
#[test]
fn test_verify_cut_through_coinbase() -> Result<(), Error> {
	global::set_local_chain_type(global::ChainTypes::UserTesting);

	let keychain = ExtKeychain::from_random_seed(false).unwrap();

	let key_id1 = ExtKeychain::derive_key_id(1, 1, 0, 0, 0);
	let key_id2 = ExtKeychain::derive_key_id(1, 2, 0, 0, 0);
	let key_id3 = ExtKeychain::derive_key_id(1, 3, 0, 0, 0);

	let builder = ProofBuilder::new(&keychain);

	let tx = build::transaction(
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

	let prev = BlockHeader::default();
	let key_id = ExtKeychain::derive_key_id(0, 0, 0, 0, 0);
	let mut block = new_block(&[tx], &keychain, &builder, &prev, &key_id);

	// The block should fail validation due to cut-through.
	assert_eq!(
		block.validate(&BlindingFactor::zero()),
		Err(Error::Transaction(transaction::Error::CutThrough))
	);

	// The block should fail lightweight "read" validation due to cut-through.
	assert_eq!(
		block.validate_read(),
		Err(Error::Transaction(transaction::Error::CutThrough))
	);

	// Apply cut-through to eliminate the offending input and output.
	let mut inputs: Vec<_> = block.inputs().into();
	let mut outputs = block.outputs().to_vec();
	let (inputs, outputs, _, _) = transaction::cut_through(&mut inputs[..], &mut outputs[..])?;

	block.body = block
		.body
		.replace_inputs(inputs.into())
		.replace_outputs(outputs);

	// Block validates successfully after applying cut-through.
	block.validate(&BlindingFactor::zero())?;

	// Block validates via lightweight "read" validation.
	block.validate_read()?;

	Ok(())
}
