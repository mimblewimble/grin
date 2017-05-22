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
    assert_eq!(block, d_block);

}

#[test]
fn should_have_block_header_codec_roundtrip() {
	use tokio_io::codec::{Encoder, Decoder};

	let mut codec = BlockCodec::default();
	let block_header = BlockHeader::default();

	let mut buf = BytesMut::with_capacity(0);
	codec.encode(block_header.clone(), &mut buf);

	let d_block_header = codec.decode(&mut buf).unwrap().unwrap();

    assert_eq!(block_header, d_block_header);

}


#[test]
fn should_encode_and_decode_input() {
	let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));

	let mut buf = BytesMut::with_capacity(0);
	input.block_encode(&mut buf);

    let d_input = Input::block_decode(&mut buf).unwrap().unwrap();

    assert_eq!(input,d_input)
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

    assert_eq!(output, d_output);
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

    assert_eq!(kernel, d_kernel);
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
