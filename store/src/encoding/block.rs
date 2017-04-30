
use std::io;
use std::io::Read;
use std::marker::PhantomData;

use tokio_io::*;
use bytes::{Bytes, BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use num_bigint::BigUint;

use core::core::{Input, Output, Proof, Transaction, TxKernel, Block, BlockHeader};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::transaction::{OutputFeatures, KernelFeatures};
use core::consensus::PROOFSIZE;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

pub struct BlockCodec;

impl codec::Encoder for BlockCodec {
	type Item = Block;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		unimplemented!()
	}
}

impl codec::Decoder for BlockCodec {
	type Item = Block;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		unimplemented!()
	}
}

/// Convenience Trait
trait BlockEncode: Sized {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Convenience Trait
trait BlockDecode: Sized {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

impl BlockEncode for Block {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		unimplemented!()
	}
}

impl BlockDecode for Block {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		unimplemented!()
	}
}

impl BlockEncode for BlockHeader {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		unimplemented!()
	}
}

impl BlockDecode for BlockHeader {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		unimplemented!()
	}
}

impl BlockEncode for Input {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(PEDERSEN_COMMITMENT_SIZE);
		dst.put_slice((self.0).0.as_ref());
		Ok(())
	}
}

impl BlockDecode for Input {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < PEDERSEN_COMMITMENT_SIZE {
			return Ok(None);
		}

		let mut buf = src.split_to(PEDERSEN_COMMITMENT_SIZE).into_buf();
		let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
		buf.copy_to_slice(&mut c);

		Ok(Some(Input(Commitment(c))))
	}
}

impl BlockEncode for Output {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(PEDERSEN_COMMITMENT_SIZE + 5134 + 1);
		dst.put_u8(self.features.bits());
		dst.put_slice(self.commit.as_ref());
		dst.put_slice(self.proof.as_ref());
		Ok(())
	}
}

impl BlockDecode for Output {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {

		let output_size = PEDERSEN_COMMITMENT_SIZE + 5134 + 1;
		if src.len() < output_size {
			return Ok(None);
		}

		let mut buf = src.split_to(output_size).into_buf();
		let feature_data = buf.get_u8();

		let mut commit_data = [0; PEDERSEN_COMMITMENT_SIZE];
		buf.copy_to_slice(&mut commit_data);

		let mut proof_data = [0; 5134];
		buf.copy_to_slice(&mut proof_data);

		Ok(Some(Output {
			features: OutputFeatures::from_bits(feature_data).unwrap(),
			commit: Commitment(commit_data),
			proof: RangeProof {
				proof: proof_data,
				plen: proof_data.len(),
			},
		}))
	}
}

impl BlockEncode for TxKernel {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(1);
		dst.put_u8(self.features.bits());

		dst.reserve(PEDERSEN_COMMITMENT_SIZE);
		dst.put_slice(self.excess.0.as_ref());

		dst.reserve(self.excess_sig.len() + 4);
		dst.put_u64::<BigEndian>(self.excess_sig.len() as u64);
		dst.put_slice(self.excess_sig.as_ref());

		dst.reserve(4);
		dst.put_u64::<BigEndian>(self.fee);

		Ok(())
	}
}

impl BlockDecode for TxKernel {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 1 + PEDERSEN_COMMITMENT_SIZE {
			return Ok(None);
		}

		let mut buf = src.split_to(1 + PEDERSEN_COMMITMENT_SIZE).into_buf();

		let features = KernelFeatures::from_bits(buf.get_u8())
			.ok_or(io::Error::new(io::ErrorKind::InvalidData, "Invalid TxKernel Feature"))?;

		let mut commit_data = [0; PEDERSEN_COMMITMENT_SIZE];
		buf.copy_to_slice(&mut commit_data);
		let commitment = Commitment(commit_data);

		if src.len() < 8 {
			return Ok(None);
		}


		let mut buf = src.split_to(8).into_buf();
		let excess_sig_len = buf.get_u64::<BigEndian>() as usize;

		if src.len() < excess_sig_len {
			return Ok(None);
		}

		let buf = src.split_to(excess_sig_len).into_buf();
		let excess_sig = buf.collect();

		if src.len() < 8 {
			return Ok(None);
		}

		let mut buf = src.split_to(8).into_buf();
		let fee = buf.get_u64::<BigEndian>();

		Ok(Some(TxKernel {
			features: features,
			excess: commitment,
			excess_sig: excess_sig,
			fee: fee,
		}))
	}
}

impl BlockEncode for Difficulty {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		let data = self.clone().into_biguint().to_bytes_be();
		dst.reserve(1 + data.len());
		dst.put_u8(data.len() as u8);
		dst.put_slice(&data);
		Ok(())
	}
}

impl BlockDecode for Difficulty {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let dlen = buf.get_u8() as usize;

		if src.len() < dlen {
			return Ok(None);
		}

		let buf = src.split_to(dlen).into_buf();
		let data = Buf::bytes(&buf);

		Ok(Some(Difficulty::from_biguint(BigUint::from_bytes_be(data))))
	}
}

impl BlockEncode for Hash {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(32);
		dst.put_slice(self.as_ref());
		Ok(())
	}
}

impl BlockDecode for Hash {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 32 {
			return Ok(None);
		}

		let mut buf = src.split_to(32).into_buf();
		let mut hash_data = [0; 32];
		buf.copy_to_slice(&mut hash_data);

		Ok(Some(Hash(hash_data)))
	}
}

impl BlockEncode for Proof {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(4 * PROOFSIZE);
		for n in 0..PROOFSIZE {
			dst.put_u32::<BigEndian>(self.0[n]);
		}
		Ok(())
	}
}

impl BlockDecode for Proof {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < 4 * PROOFSIZE {
			return Ok(None);
		}
		let mut buf = src.split_to(4 * PROOFSIZE).into_buf();
		let mut proof_data = [0u32; PROOFSIZE];
		for n in 0..PROOFSIZE {
			proof_data[n] = buf.get_u32::<BigEndian>();
		}
		Ok(Some(Proof(proof_data)))
	}
}

#[test]
fn should_have_block_codec_roundtrip() {}

#[test]
fn should_encode_and_decode_block() {

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
	block.block_encode(&mut buf);

	let d_block = Block::block_decode(&mut buf).unwrap().unwrap();

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
fn should_encode_and_decode_blockheader() {

	let block_header = BlockHeader::default();

	let mut buf = BytesMut::with_capacity(0);
	block_header.block_encode(&mut buf);

	let d_block_header = BlockHeader::block_decode(&mut buf).unwrap().unwrap();

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