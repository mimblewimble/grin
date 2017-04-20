
use std::io;
use std::marker::PhantomData;

use tokio_io::*;
use bytes::{Bytes, BytesMut};

use core::core::{Input, Output, Proof, Transaction, TxKernel, Block, BlockHeader};
use secp::pedersen::{RangeProof, Commitment};

struct TransactionCodec {}
