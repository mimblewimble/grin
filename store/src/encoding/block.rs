
use std::io;
use std::marker::PhantomData;

use tokio_io::*;
use bytes::{Bytes, BytesMut, BufMut};

use core::core::{Input, Output, Proof, Transaction, TxKernel, Block, BlockHeader};
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

