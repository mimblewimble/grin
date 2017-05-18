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

use super::tx::*;

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
		excess_sig: vec![0; 10],
	};

	let mut buf = BytesMut::with_capacity(0);
	let mut codec = TxCodec {};
	codec
		.encode(tx.clone(), &mut buf)
		.expect("Error During Transaction Encoding");

	let d_tx = codec
		.decode(&mut buf)
		.expect("Error During Transaction Decoding")
		.expect("Unfinished Transaction");

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
