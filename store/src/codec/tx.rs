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

use core::core::{Input, Output, Transaction};
use core::core::transaction::OutputFeatures;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

// Convenience Macro for Option Handling in Decoding
macro_rules! try_opt_dec {
	($e: expr) => (match $e {
		Some(val) => val,
		None => return Ok(None),
	});
}

/// Decodes and encodes `Transaction`s
#[derive(Debug, Clone, Default)]
pub struct TxCodec;

impl codec::Encoder for TxCodec {
	type Item = Transaction;
	type Error = io::Error;
	fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
		// Put Fee
		dst.reserve(8);
		dst.put_u64::<BigEndian>(item.fee);

		// Put Excess Sig Length as u64 + Excess Sig Itself
		dst.reserve(8 + item.excess_sig.len());
		dst.put_u64::<BigEndian>(item.excess_sig.len() as u64);
		dst.put_slice(&item.excess_sig);

		// Put Inputs and Outputs Lengths as 2 u64's
		dst.reserve(16);
		dst.put_u64::<BigEndian>(item.inputs.len() as u64);
		dst.put_u64::<BigEndian>(item.outputs.len() as u64);

		// Put Inputs
		for inp in &item.inputs {
			inp.tx_encode(dst)?;
		}

		// Put Outputs
		for out in &item.outputs {
			out.tx_encode(dst)?;
		}

		Ok(())
	}
}

impl codec::Decoder for TxCodec {
	type Item = Transaction;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Get Fee
		if src.len() < 8 {
			return Ok(None);
		}
		let mut buf = src.split_to(8).into_buf();
		let fee = buf.get_u64::<BigEndian>();

		// Get Excess Sig Length
		if src.len() < 8 {
			return Ok(None);
		}
		let mut buf = src.split_to(8).into_buf();
		let excess_sig_len = buf.get_u64::<BigEndian>() as usize;

		// Get Excess Sig
		if src.len() < excess_sig_len {
			return Ok(None);
		}
		let buf = src.split_to(excess_sig_len).into_buf();
		let excess_sig = buf.collect();

		// Get Inputs and Outputs Lengths from 2 u64's
		if src.len() < 16 {
			return Ok(None);
		}
		let mut buf = src.split_to(16).into_buf();
		let inputs_len = buf.get_u64::<BigEndian>() as usize;
		let outputs_len = buf.get_u64::<BigEndian>() as usize;

		// Get Inputs
		let mut inputs = Vec::with_capacity(inputs_len);
		for _ in 0..inputs_len {
			inputs.push(try_opt_dec!(Input::tx_decode(src)?));
		}

		// Get Outputs
		let mut outputs = Vec::with_capacity(outputs_len);
		for _ in 0..outputs_len {
			outputs.push(try_opt_dec!(Output::tx_decode(src)?));
		}

		Ok(Some(Transaction {
			fee: fee,
			excess_sig: excess_sig,
			inputs: inputs,
			outputs: outputs,
		}))
	}
}

/// Internal Convenience Trait
pub trait TxEncode: Sized {
	fn tx_encode(&self, dst: &mut BytesMut) -> Result<(), io::Error>;
}

/// Internal Convenience Trait
pub trait TxDecode: Sized {
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