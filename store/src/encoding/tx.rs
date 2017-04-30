
use std::io;

use tokio_io::*;
use bytes::{Bytes, BytesMut, Buf, BufMut, IntoBuf};

use core::core::{Input, Output, Proof, Transaction, TxKernel, Block, BlockHeader};
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

pub struct TxCodec;

impl codec::Encoder for TxCodec {
	type Item = Transaction;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		unimplemented!()
	}
}

impl codec::Decoder for TxCodec {
	type Item = Transaction;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		unimplemented!()
	}
}

/// Internal Convenience Trait
trait TxEncode: Sized {
	fn tx_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Internal Convenience Trait
trait TxDecode: Sized {
	fn tx_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error>;
}

impl TxEncode for Output {
	fn tx_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(PEDERSEN_COMMITMENT_SIZE + 5134 + 1);
		dst.put_u8(self.features.bits());
		dst.put_slice(self.commit.as_ref());
		dst.put_slice(self.proof.as_ref());
		Ok(())
	}
}

impl TxDecode for Output {
	fn tx_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
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

impl TxEncode for Input {
	fn tx_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error> {
		dst.reserve(PEDERSEN_COMMITMENT_SIZE);
		dst.put_slice((self.0).0.as_ref());
		Ok(())
	}
}

impl TxDecode for Input {
	fn tx_decode(src: &mut BytesMut) -> Result<Option<Self>, io::Error> {
		if src.len() < PEDERSEN_COMMITMENT_SIZE {
			return Ok(None);
		}

		let mut buf = src.split_to(PEDERSEN_COMMITMENT_SIZE).into_buf();
		let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
		buf.copy_to_slice(&mut c);

		Ok(Some(Input(Commitment(c))))
	}
}




#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn should_have_tx_codec_roundtrip() {
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

		let tx = Transaction {
			inputs: vec![input],
			outputs: vec![output],
			fee: 0,
			excess_sig: vec![0; 10]
		};

		let mut buf = BytesMut::with_capacity(0);
		let mut codec = TxCodec {};
		codec.encode(tx.clone(), &mut buf).expect("Error During Transaction Encoding");

		let d_tx =
			codec.decode(&mut buf).expect("Error During Transaction Decoding").expect("Unfinished Transaction");
		
		assert_eq!(tx.inputs[0].commitment(), d_tx.inputs[0].commitment());
		assert_eq!(tx.outputs[0].features, d_tx.outputs[0].features);
		assert_eq!(tx.fee, d_tx.fee);
		assert_eq!(tx.excess_sig, d_tx.excess_sig);
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
		output.tx_encode(&mut buf);

		let d_output = Output::tx_decode(&mut buf).unwrap().unwrap();

		assert_eq!(output.features, d_output.features);
		assert_eq!(output.proof().as_ref(), d_output.proof().as_ref());
		assert_eq!(output.commitment(), d_output.commitment());

	}

	#[test]
	fn should_encode_and_decode_input() {
		let input = Input(Commitment([1; PEDERSEN_COMMITMENT_SIZE]));

		let mut buf = BytesMut::with_capacity(0);
		input.tx_encode(&mut buf);

		assert_eq!([1; PEDERSEN_COMMITMENT_SIZE].as_ref(), buf);
		assert_eq!(input.commitment(),
		           Input::tx_decode(&mut buf)
			           .unwrap()
			           .unwrap()
			           .commitment());
	}
}
