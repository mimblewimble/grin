
use std::io;
use std::marker::PhantomData;

use tokio_io::*;
use bytes::{Bytes, BytesMut, BufMut};

use core::core::{Input, Output, Proof, Transaction, TxKernel, Block, BlockHeader};
use core::core::transaction::OutputFeatures;
use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

pub struct BlockCodec<T: BlockEncode + BlockDecode> {
	marker: PhantomData<T>,
}

impl<T: BlockEncode + BlockDecode> codec::Encoder for BlockCodec<T> {
	type Item = T;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		Ok(())
	}
}

impl<T: BlockEncode + BlockDecode> codec::Decoder for BlockCodec<T> {
	type Item = T;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		Ok(None)
	}
}

pub trait BlockEncode: Sized {
	fn block_encode(&self, dst: &mut BytesMut);
}

pub trait BlockDecode: Sized {
	fn block_decode(src: Bytes) -> io::Result<Self>;
}

impl BlockEncode for Input {
	fn block_encode(&self, dst: &mut BytesMut) {
		dst.put_slice((self.0).0.as_ref())
	}
}

impl BlockDecode for Input {
	fn block_decode(src: Bytes) -> io::Result<Self> {
		if let Some(s) = src.get(0..PEDERSEN_COMMITMENT_SIZE) {
			let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
			for i in 0..PEDERSEN_COMMITMENT_SIZE {
				c[i] = s[i];
			}
			Ok(Input(Commitment(c)))
		} else {
			Err(io::Error::from(io::ErrorKind::InvalidData))
		}
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
		let (feature_data, remaining) = src.split_at(0);
		let (commit_data, proof_data) = remaining.split_at(PEDERSEN_COMMITMENT_SIZE);
		if proof_data.len() == 0 {
			return Err(io::Error::from(io::ErrorKind::InvalidData));
		}
		let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
		for i in 0..PEDERSEN_COMMITMENT_SIZE {
			c[i] = commit_data[i];
		}

		let mut p = [0; 5134];
		for i in 0..proof_data.len() {
			p[i] = proof_data[i];
		}
		Ok(Output {
		       features: OutputFeatures::from_bits(feature_data[0]).unwrap(),
		       commit: Commitment(c),
		       proof: RangeProof {
		           proof: p,
		           plen: proof_data.len(),
		       },
		   })
	}
}
