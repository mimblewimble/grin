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

use core::core::{Input, Output, Proof, TxKernel, Block, BlockHeader};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::transaction::{OutputFeatures, KernelFeatures};
use core::core::block::BlockFeatures;
use core::consensus::PROOFSIZE;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

#[derive(Debug, Clone)]
pub struct BlockCodec;

impl codec::Encoder for BlockCodec {
	type Item = Block;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		// Put Header
		item.header.block_encode(dst)?;

		// Put Lengths of Inputs, Outputs and Kernels in 3 u64's
		dst.reserve(24);
		dst.put_u64::<BigEndian>(item.inputs.len() as u64);
		dst.put_u64::<BigEndian>(item.outputs.len() as u64);
		dst.put_u64::<BigEndian>(item.kernels.len() as u64);

		// Put Inputs
		for inp in &item.inputs {
			inp.block_encode(dst)?;
		}

		// Put Outputs
		for outp in &item.outputs {
			outp.block_encode(dst)?;
		}

		// Put TxKernels
		for proof in &item.kernels {
			proof.block_encode(dst)?;
		}

		Ok(())
	}
}

impl codec::Decoder for BlockCodec {
	type Item = Block;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Get Header
		let header = try_opt_dec!(BlockHeader::block_decode(src)?);

		// Get Lengths of Inputs, Outputs and Kernels from 3 u64's
		if src.len() < 24 {
			return Ok(None);
		}
		let mut buf = src.split_to(24).into_buf();
		let inputs_len = buf.get_u64::<BigEndian>() as usize;
		let outputs_len = buf.get_u64::<BigEndian>() as usize;
		let kernels_len = buf.get_u64::<BigEndian>() as usize;

		// Get Inputs
		let mut inputs = Vec::with_capacity(inputs_len);
		for _ in 0..inputs_len {
			inputs.push(try_opt_dec!(Input::block_decode(src)?));
		}

		// Get Outputs
		let mut outputs = Vec::with_capacity(outputs_len);
		for _ in 0..outputs_len {
			outputs.push(try_opt_dec!(Output::block_decode(src)?));
		}

		// Get Kernels
		let mut kernels = Vec::with_capacity(kernels_len);
		for _ in 0..kernels_len {
			kernels.push(try_opt_dec!(TxKernel::block_decode(src)?));
		}

		Ok(Some(Block {
			header: header,
			inputs: inputs,
			outputs: outputs,
			kernels: kernels,
		}))

	}
}

#[derive(Debug, Clone)]
pub struct BlockHasher;

impl codec::Encoder for BlockHasher {
	type Item = Block;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		// Only encode header
		partial_block_encode(&item.header, dst)
	}
}

/// Internal Convenience Trait
trait BlockEncode: Sized {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Internal Convenience Trait
trait BlockDecode: Sized {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

impl BlockEncode for BlockHeader {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		partial_block_encode(self, dst)?;

		// Put Proof of Work Data
		self.pow.block_encode(dst)?;
		Ok(())
	}
}

fn partial_block_encode(header: &BlockHeader, dst: &mut BytesMut) -> Result<(), io::Error> {
	// Put Height
	dst.reserve(8);
	dst.put_u64::<BigEndian>(header.height);

	// Put Previous Hash
	header.previous.block_encode(dst)?;

	// Put Timestamp
	dst.reserve(8);
	dst.put_i64::<BigEndian>(header.timestamp.to_timespec().sec);

	// Put Cuckoo Len
	dst.reserve(1);
	dst.put_u8(header.cuckoo_len);

	// Put UTXO Merkle Hash
	header.utxo_merkle.block_encode(dst)?;

	// Put Merkle Tree Hashes
	header.tx_merkle.block_encode(dst)?;

	// Put Features
	dst.reserve(1);
	dst.put_u8(header.features.bits());

	// Put Nonce
	dst.reserve(8);
	dst.put_u64::<BigEndian>(header.nonce);

	// Put Difficulty
	header.difficulty.block_encode(dst)?;

	// Put Total Difficulty
	header.total_difficulty.block_encode(dst)?;

	Ok(())
}

impl BlockDecode for BlockHeader {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		// Get Height
		if src.len() < 8 {
			return Ok(None);
		}
		let mut buf = src.split_to(8).into_buf();
		let height = buf.get_u64::<BigEndian>();

		// Get Previous Hash
		let previous = try_opt_dec!(Hash::block_decode(src)?);

		// Get Timestamp
		if src.len() < 8 {
			return Ok(None);
		}
		let mut buf = src.split_to(8).into_buf();
		let timestamp = time::at_utc(Timespec {
			sec: buf.get_i64::<BigEndian>(),
			nsec: 0,
		});

		// Get Cuckoo Len
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let cuckoo_len = buf.get_u8();

		// Get UTXO Merkle Hash
		let utxo_merkle = try_opt_dec!(Hash::block_decode(src)?);

		// Get Merkle Tree Hashes
		let tx_merkle = try_opt_dec!(Hash::block_decode(src)?);

		// Get Features
		if src.len() < 1 {
			return Ok(None);
		}
		let mut buf = src.split_to(1).into_buf();
		let features = BlockFeatures::from_bits(buf.get_u8())
			.ok_or(io::Error::new(io::ErrorKind::InvalidData, "Invalid BlockHeader Feature"))?;

		// Get Nonce
		if src.len() < 8 {
			return Ok(None);
		}
		let mut buf = src.split_to(8).into_buf();
		let nonce = buf.get_u64::<BigEndian>();

		// Get Difficulty
		let difficulty = try_opt_dec!(Difficulty::block_decode(src)?);

		// Get Total Difficulty
		let total_difficulty = try_opt_dec!(Difficulty::block_decode(src)?);

		// Get Proof of Work Data
		let pow = try_opt_dec!(Proof::block_decode(src)?);

		Ok(Some(BlockHeader {
			height: height,
			previous: previous,
			timestamp: timestamp,
			cuckoo_len: cuckoo_len,
			utxo_merkle: utxo_merkle,
			tx_merkle: tx_merkle,
			features: features,
			nonce: nonce,
			pow: pow,
			difficulty: difficulty,
			total_difficulty: total_difficulty,
		}))
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

#[cfg(test)]
mod tests {
	use super::*;

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
		let mut codec = BlockCodec {};
		codec.encode(block.clone(), &mut buf).expect("Error During Block Encoding");

		let d_block =
			codec.decode(&mut buf).expect("Error During Block Decoding").expect("Unfinished Block");

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
}