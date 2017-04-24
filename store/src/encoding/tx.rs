
use std::io;

use tokio_io::*;
use bytes::{Bytes, BytesMut, Buf, BufMut, IntoBuf};

use core::core::Transaction;

struct TxCodec;

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

#[test]
fn should_have_tx_codec_roundtrip() { unimplemented!() }
