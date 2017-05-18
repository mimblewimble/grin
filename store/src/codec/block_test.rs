// Copyright 2016 The Grin Developers
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

use std::io;

use tokio_io::*;
use bytes::{BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use num_bigint::BigUint;
use time::Timespec;
use time;
use std::marker::PhantomData;

use core::core::{Input, Output, Proof, TxKernel, Block, BlockHeader};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::transaction::{OutputFeatures, KernelFeatures};
use core::core::block::BlockFeatures;
use core::consensus::PROOFSIZE;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

use super::block::*;

#[test]
fn should_have_block_codec_roundtrip() {
	use tokio_io::codec::{Encoder, Decoder};

	let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));

	let output = Output {
		features: OutputFeatures::empty(),
		commit: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
		proof: RangeProof {
			proof: [1; 5134],
			plen: 5134,
		},
	};

	let kernel = TxKernel {
		features: KernelFeatures::empty(),
		excess: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
		excess_sig: vec![1; 10],
		fee: 100,
	};

	let block = Block {
		header: BlockHeader::default(),
		inputs: vec![input],
		outputs: vec![output],
		kernels: vec![kernel],
	};

	let mut buf = BytesMut::with_capacity(0);
	let mut codec = BlockCodec::default();
	codec
		.encode(block.clone(), &mut buf)
		.expect("Error During Block Encoding");

	let d_block = codec
		.decode(&mut buf)
		.expect("Error During Block Decoding")
		.expect("Unfinished Block");

	// Check if all bytes are read
	assert_eq!(buf.len(), 0);

	assert_eq!(block.header.height, d_block.header.height);
	assert_eq!(block.header.previous, d_block.header.previous);
	assert_eq!(block.header.timestamp, d_block.header.timestamp);
	assert_eq!(block.header.cuckoo_len, d_block.header.cuckoo_len);
	assert_eq!(block.header.utxo_merkle, d_block.header.utxo_merkle);
	assert_eq!(block.header.tx_merkle, d_block.header.tx_merkle);
	assert_eq!(block.header.features, d_block.header.features);
	assert_eq!(block.header.nonce, d_block.header.nonce);
	assert_eq!(block.header.pow, d_block.header.pow);
	assert_eq!(block.header.difficulty, d_block.header.difficulty);
	assert_eq!(block.header.total_difficulty,
	           d_block.header.total_difficulty);

	assert_eq!(block.inputs[0].commitment(), d_block.inputs[0].commitment());

	assert_eq!(block.outputs[0].features, d_block.outputs[0].features);
	assert_eq!(block.outputs[0].proof().as_ref(),
	           d_block.outputs[0].proof().as_ref());
	assert_eq!(block.outputs[0].commitment(),
	           d_block.outputs[0].commitment());

	assert_eq!(block.kernels[0].features, d_block.kernels[0].features);
	assert_eq!(block.kernels[0].excess, d_block.kernels[0].excess);
	assert_eq!(block.kernels[0].excess_sig, d_block.kernels[0].excess_sig);
	assert_eq!(block.kernels[0].fee, d_block.kernels[0].fee);

}

#[test]
fn should_have_block_header_codec_roundtrip() {
	use tokio_io::codec::{Encoder, Decoder};

	let mut codec = BlockCodec::default();
	let block_header = BlockHeader::default();

	let mut buf = BytesMut::with_capacity(0);
	codec.encode(block_header.clone(), &mut buf);

	let d_block_header = codec.decode(&mut buf).unwrap().unwrap();

	assert_eq!(block_header.height, d_block_header.height);
	assert_eq!(block_header.previous, d_block_header.previous);
	assert_eq!(block_header.timestamp, d_block_header.timestamp);
	assert_eq!(block_header.cuckoo_len, d_block_header.cuckoo_len);
	assert_eq!(block_header.utxo_merkle, d_block_header.utxo_merkle);
	assert_eq!(block_header.tx_merkle, d_block_header.tx_merkle);
	assert_eq!(block_header.features, d_block_header.features);
	assert_eq!(block_header.nonce, d_block_header.nonce);
	assert_eq!(block_header.pow, d_block_header.pow);
	assert_eq!(block_header.difficulty, d_block_header.difficulty);
	assert_eq!(block_header.total_difficulty,
	           d_block_header.total_difficulty);

}


#[test]
fn should_encode_and_decode_input() {
	let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));

	let mut buf = BytesMut::with_capacity(0);
	input.block_encode(&mut buf);

	assert_eq!([1; PEDERSEN_COMMITMENT_SIZE].as_ref(), buf);
	assert_eq!(input.commitment(),
	           Input::block_decode(&mut buf)
	               .unwrap()
	               .unwrap()
	               .commitment());
}

#[test]
fn should_encode_and_decode_output() {
	let output = Output {
		features: OutputFeatures::empty(),
		commit: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
		proof: RangeProof {
			proof: [1; 5134],
			plen: 5134,
		},
	};

	let mut buf = BytesMut::with_capacity(0);
	output.block_encode(&mut buf);

	let d_output = Output::block_decode(&mut buf).unwrap().unwrap();

	assert_eq!(output.features, d_output.features);
	assert_eq!(output.proof().as_ref(), d_output.proof().as_ref());
	assert_eq!(output.commitment(), d_output.commitment());

}

#[test]
fn should_encode_and_decode_txkernel() {

	let kernel = TxKernel {
		features: KernelFeatures::empty(),
		excess: Commitment([1; PEDERSEN_COMMITMENT_SIZE]),
		excess_sig: vec![1; 10],
		fee: 100,
	};

	let mut buf = BytesMut::with_capacity(0);
	kernel.block_encode(&mut buf);

	let d_kernel = TxKernel::block_decode(&mut buf).unwrap().unwrap();

	assert_eq!(kernel.features, d_kernel.features);
	assert_eq!(kernel.excess, d_kernel.excess);
	assert_eq!(kernel.excess_sig, d_kernel.excess_sig);
	assert_eq!(kernel.fee, d_kernel.fee);
}

#[test]
fn should_encode_and_decode_difficulty() {

	let difficulty = Difficulty::from_num(1000);

	let mut buf = BytesMut::with_capacity(0);
	difficulty.block_encode(&mut buf);

	let d_difficulty = Difficulty::block_decode(&mut buf).unwrap().unwrap();

	assert_eq!(difficulty, d_difficulty);
}

#[test]
fn should_encode_and_decode_hash() {

	let hash = Hash([1u8; 32]);

	let mut buf = BytesMut::with_capacity(0);
	hash.block_encode(&mut buf);

	let d_hash = Hash::block_decode(&mut buf).unwrap().unwrap();

	assert_eq!(hash, d_hash);
}

#[test]
fn should_encode_and_decode_proof() {

	let proof = Proof::zero();

	let mut buf = BytesMut::with_capacity(0);
	proof.block_encode(&mut buf);

	let d_proof = Proof::block_decode(&mut buf).unwrap().unwrap();

	assert_eq!(proof, d_proof);
}
