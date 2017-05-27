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

use core::{Input, Output, Transaction};
use codec::block::{BlockEncode, BlockDecode};

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
			inp.block_encode(dst)?;
		}

		// Put Outputs
		for out in &item.outputs {
			out.block_encode(dst)?;
		}

		Ok(())
	}
}

impl codec::Decoder for TxCodec {
	type Item = Transaction;
	type Error = io::Error;
	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		// Create Temporary Buffer
		let ref mut temp = src.clone();

		// Get Fee
		if temp.len() < 8 {
			return Ok(None);
		}
		let mut buf = temp.split_to(8).into_buf();
		let fee = buf.get_u64::<BigEndian>();

		// Get Excess Sig Length
		if temp.len() < 8 {
			return Ok(None);
		}
		let mut buf = temp.split_to(8).into_buf();
		let excess_sig_len = buf.get_u64::<BigEndian>() as usize;

		// Get Excess Sig
		if temp.len() < excess_sig_len {
			return Ok(None);
		}
		let buf = temp.split_to(excess_sig_len).into_buf();
		let excess_sig = buf.collect();

		// Get Inputs and Outputs Lengths from 2 u64's
		if temp.len() < 16 {
			return Ok(None);
		}
		let mut buf = temp.split_to(16).into_buf();
		let inputs_len = buf.get_u64::<BigEndian>() as usize;
		let outputs_len = buf.get_u64::<BigEndian>() as usize;

		// Get Inputs
		let mut inputs = Vec::with_capacity(inputs_len);
		for _ in 0..inputs_len {
			inputs.push(try_opt_dec!(Input::block_decode(temp)?));
		}

		// Get Outputs
		let mut outputs = Vec::with_capacity(outputs_len);
		for _ in 0..outputs_len {
			outputs.push(try_opt_dec!(Output::block_decode(temp)?));
		}

		// If succesfull truncate src by bytes read from src;
		let diff = src.len() - temp.len();
		src.split_to(diff);

		Ok(Some(Transaction {
			fee: fee,
			excess_sig: excess_sig,
			inputs: inputs,
			outputs: outputs,
		}))
	}
}