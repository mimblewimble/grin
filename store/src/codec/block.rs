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

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

/// Internal Convenience Trait
pub trait BlockEncode: Sized {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Internal Convenience Trait
pub trait BlockDecode: Sized {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

/// Decodes and encodes `Block`s and their subtypes
#[derive(Debug, Clone)]
pub struct BlockCodec<T: BlockDecode + BlockEncode> {
	phantom: PhantomData<T>,
}

impl<T> Default for BlockCodec<T>
    where T: BlockDecode + BlockEncode
{
	fn default() -> Self {
		BlockCodec { phantom: PhantomData }
	}
}

impl<T> codec::Encoder for BlockCodec<T>
    where T: BlockDecode + BlockEncode
{
	type Item = T;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		T::block_encode(&item, dst)
	}
}

impl<T> codec::Decoder for BlockCodec<T>
    where T: BlockDecode + BlockEncode
{
	type Item = T;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Create Temporary Buffer
		let ref mut temp = src.clone();
		let res = try_opt_dec!(T::block_decode(temp)?);

		// If succesfull truncate src by bytes read from src;
		let diff = src.len() - temp.len();
		src.split_to(diff);

		// Return Item
		Ok(Some(res))
	}
}


impl BlockEncode for Block {
	fn block_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		// Put Header
		self.header.block_encode(dst)?;

		// Put Lengths of Inputs, Outputs and Kernels in 3 u64's
		dst.reserve(24);
		dst.put_u64::<BigEndian>(self.inputs.len() as u64);
		dst.put_u64::<BigEndian>(self.outputs.len() as u64);
		dst.put_u64::<BigEndian>(self.kernels.len() as u64);

		// Put Inputs
		for inp in &self.inputs {
			inp.block_encode(dst)?;
		}

		// Put Outputs
		for outp in &self.outputs {
			outp.block_encode(dst)?;
		}

		// Put TxKernels
		for proof in &self.kernels {
			proof.block_encode(dst)?;
		}

		Ok(())
	}
}

impl BlockDecode for Block {
	fn block_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {

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
