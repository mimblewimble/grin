
use std::io;
use std::io::Read;
use std::marker::PhantomData;

use tokio_io::*;
use bytes::{Bytes, BytesMut, BufMut, Buf, IntoBuf};

use core::core::{Input, Output, Proof, Transaction, TxKernel, Block, BlockHeader};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::transaction::{OutputFeatures, KernelFeatures};
use core::consensus::PROOFSIZE;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

pub struct BlockCodec<T: BlockEncode + BlockDecode> {
	marker: PhantomData<T>,
}

impl<T: BlockEncode + BlockDecode> codec::Encoder for BlockCodec<T> {
	type Item = T;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		unimplemented!()
	}
}

impl<T: BlockEncode + BlockDecode> codec::Decoder for BlockCodec<T> {
	type Item = T;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		unimplemented!()
	}
}

pub trait BlockEncode: Sized {
	fn block_encode(&self, dst: &mut BytesMut);
}

pub trait BlockDecode: Sized {
	fn block_decode(src: Bytes) -> io::Result<Self>;
}

impl BlockEncode for Block {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for Block {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

impl BlockEncode for BlockHeader {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for BlockHeader {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

impl BlockEncode for Input {
	fn block_encode(&self, dst: &mut BytesMut) {
		dst.put_slice((self.0).0.as_ref())
	}
}

impl BlockDecode for Input {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		let mut reader = src.into_buf().reader();
		let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
		reader.read_exact(&mut c)?;
		Ok(Input(Commitment(c)))
	}
}

impl BlockEncode for Output {
	fn block_encode(&self, dst: &mut BytesMut) {
		dst.put_u8(self.features.bits());
		dst.put_slice(self.commit.as_ref());
		dst.put_slice(self.proof.as_ref());
	}
}

impl BlockDecode for Output {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		let mut buf = src.into_buf();
		let feature_data = buf.get_u8();

		let mut reader = buf.reader();

		let mut commit_data = [0; PEDERSEN_COMMITMENT_SIZE];
		reader.read_exact(&mut commit_data)?;

		let mut proof_data = [0; 5134];
		reader.read_exact(&mut proof_data)?;

		Ok(Output {
			features: OutputFeatures::from_bits(feature_data).unwrap(),
			commit: Commitment(commit_data),
			proof: RangeProof {
				proof: proof_data,
				plen: proof_data.len(),
			},
		})
	}
}

impl BlockEncode for TxKernel {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for TxKernel {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

impl BlockEncode for Difficulty {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for Difficulty {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

impl BlockEncode for Hash {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for Hash {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

impl BlockEncode for Proof {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for Proof {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

impl BlockEncode for RangeProof {
	fn block_encode(&self, dst: &mut BytesMut) {
		unimplemented!()
	}
}

impl BlockDecode for RangeProof {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		unimplemented!()
	}
}

#[test]
fn should_have_block_codec_roundtrip() { unimplemented!() }

#[test]
fn should_encode_and_decode_block() { unimplemented!() }

#[test]
fn should_encode_and_decode_blockheader() { unimplemented!() }


#[test]
fn should_encode_and_decode_input() {
	let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));

	let mut buf = BytesMut::with_capacity(PEDERSEN_COMMITMENT_SIZE);
	input.block_encode(&mut buf);

	assert_eq!([1; PEDERSEN_COMMITMENT_SIZE].as_ref(), buf);
	assert_eq!(input.commitment(),
	           Input::block_decode(buf.freeze()).unwrap().commitment());
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

	let mut buf = BytesMut::with_capacity(6000);
	output.block_encode(&mut buf);

	let d_output = Output::block_decode(buf.freeze()).unwrap();

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

	let mut buf = BytesMut::with_capacity(6000);
	kernel.block_encode(&mut buf);

	let d_kernel = TxKernel::block_decode(buf.freeze()).unwrap();

	assert_eq!(kernel.features, d_kernel.features);
	assert_eq!(kernel.excess, d_kernel.excess);
	assert_eq!(kernel.excess_sig, d_kernel.excess_sig);
	assert_eq!(kernel.fee, d_kernel.fee);
}

#[test]
fn should_encode_and_decode_difficulty() { unimplemented!() }

#[test]
fn should_encode_and_decode_hash() { unimplemented!() }

#[test]
fn should_encode_and_decode_proof() { unimplemented!() }

#[test]
fn should_encode_and_decode_rangeproof() { unimplemented!() }


