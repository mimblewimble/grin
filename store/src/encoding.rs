use std::io;

use tokio_io::*;
use bytes::{Bytes, BytesMut};

use core::core::{Input,Output,Proof,Transaction,TxKernel,Block,BlockHeader};
use core::core::target::Difficulty;

trait BitEncode: Sized {
    fn bit_encode(&self, dst: &mut BytesMut);
}

trait BitDecode: Sized {
    fn bit_decode(&self, src: Bytes) -> io::Result<Self>;
}

struct BitCodec {}

impl codec::Encoder for BitCodec {
    type Item = Block;
    type Error = io::Error;
    fn encode(&mut self, item: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl codec::Decoder for BitCodec {
    type Item = Block;
    type Error = io::Error;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(None)
    }
}
struct TransactionCodec {}


