
use std::io;
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6, Ipv4Addr, Ipv6Addr, IpAddr};

use tokio_io::*;
use bytes::{BytesMut, BigEndian, BufMut, Buf, IntoBuf};
use tokio_io::codec::{Encoder, Decoder};
use enum_primitive::FromPrimitive;

use core::core::{Block, BlockHeader, Input, Output, Transaction, TxKernel};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::transaction::{OutputFeatures, KernelFeatures};
use types::*;

use secp::pedersen::{RangeProof, Commitment};
use secp::constants::PEDERSEN_COMMITMENT_SIZE;

use grin_store::codec::{BlockCodec, TxCodec};

use peer::*;
use msg::MsgHeader;

const MSG_HEADER_SIZE: usize = 11;
const SOCKET_ADDR_MARKER_V4: u8 = 0;
const SOCKET_ADDR_MARKER_V6: u8 = 1;