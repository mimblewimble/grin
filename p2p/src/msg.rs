// Copyright 2018 The Grin Developers
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

//! Message types that transit over the network and related serialization code.

use num::FromPrimitive;
use std::io::{Read, Write};
use std::time;

use crate::core::core::hash::Hash;
use crate::core::core::BlockHeader;
use crate::core::pow::Difficulty;
use crate::core::ser::{self, FixedLength, Readable, Reader, StreamingReader, Writeable, Writer};
use crate::core::{consensus, global};
use crate::types::{
	Capabilities, Error, PeerAddr, ReasonForBan, MAX_BLOCK_HEADERS, MAX_LOCATORS, MAX_PEER_ADDRS,
};
use crate::util::read_write::read_exact;

/// Current latest version of the protocol
pub const PROTOCOL_VERSION: u32 = 1;

/// Grin's user agent with current version
pub const USER_AGENT: &'static str = concat!("MW/Grin ", env!("CARGO_PKG_VERSION"));

/// Magic numbers expected in the header of every message
const OTHER_MAGIC: [u8; 2] = [73, 43];
const FLOONET_MAGIC: [u8; 2] = [83, 59];
const MAINNET_MAGIC: [u8; 2] = [97, 61];

/// Types of messages.
/// Note: Values here are *important* so we should only add new values at the
/// end.
enum_from_primitive! {
	#[derive(Debug, Clone, Copy, PartialEq)]
	pub enum Type {
		Error = 0,
		Hand = 1,
		Shake = 2,
		Ping = 3,
		Pong = 4,
		GetPeerAddrs = 5,
		PeerAddrs = 6,
		GetHeaders = 7,
		Header = 8,
		Headers = 9,
		GetBlock = 10,
		Block = 11,
		GetCompactBlock = 12,
		CompactBlock = 13,
		StemTransaction = 14,
		Transaction = 15,
		TxHashSetRequest = 16,
		TxHashSetArchive = 17,
		BanReason = 18,
		GetTransaction = 19,
		TransactionKernel = 20,
	}
}

/// Max theoretical size of a block filled with outputs.
fn max_block_size() -> u64 {
	(global::max_block_weight() / consensus::BLOCK_OUTPUT_WEIGHT * 708) as u64
}

// Max msg size for each msg type.
fn max_msg_size(msg_type: Type) -> u64 {
	match msg_type {
		Type::Error => 0,
		Type::Hand => 128,
		Type::Shake => 88,
		Type::Ping => 16,
		Type::Pong => 16,
		Type::GetPeerAddrs => 4,
		Type::PeerAddrs => 4 + (1 + 16 + 2) * MAX_PEER_ADDRS as u64,
		Type::GetHeaders => 1 + 32 * MAX_LOCATORS as u64,
		Type::Header => 365,
		Type::Headers => 2 + 365 * MAX_BLOCK_HEADERS as u64,
		Type::GetBlock => 32,
		Type::Block => max_block_size(),
		Type::GetCompactBlock => 32,
		Type::CompactBlock => max_block_size() / 10,
		Type::StemTransaction => max_block_size(),
		Type::Transaction => max_block_size(),
		Type::TxHashSetRequest => 40,
		Type::TxHashSetArchive => 64,
		Type::BanReason => 64,
		Type::GetTransaction => 32,
		Type::TransactionKernel => 32,
	}
}

fn magic() -> [u8; 2] {
	match *global::CHAIN_TYPE.read() {
		global::ChainTypes::Floonet => FLOONET_MAGIC,
		global::ChainTypes::Mainnet => MAINNET_MAGIC,
		_ => OTHER_MAGIC,
	}
}

/// Read a header from the provided stream without blocking if the
/// underlying stream is async. Typically headers will be polled for, so
/// we do not want to block.
pub fn read_header(stream: &mut dyn Read, msg_type: Option<Type>) -> Result<MsgHeader, Error> {
	let mut head = vec![0u8; MsgHeader::LEN];
	if Some(Type::Hand) == msg_type {
		read_exact(stream, &mut head, time::Duration::from_millis(10), true)?;
	} else {
		read_exact(stream, &mut head, time::Duration::from_secs(10), false)?;
	}
	let header = ser::deserialize::<MsgHeader>(&mut &head[..])?;
	let max_len = max_msg_size(header.msg_type);

	// TODO 4x the limits for now to leave ourselves space to change things
	if header.msg_len > max_len * 4 {
		error!(
			"Too large read {}, had {}, wanted {}.",
			header.msg_type as u8, max_len, header.msg_len
		);
		return Err(Error::Serialization(ser::Error::TooLargeReadErr));
	}
	Ok(header)
}

/// Read a single item from the provided stream, always blocking until we
/// have a result (or timeout).
/// Returns the item and the total bytes read.
pub fn read_item<T: Readable>(stream: &mut dyn Read) -> Result<(T, u64), Error> {
	let timeout = time::Duration::from_secs(20);
	let mut reader = StreamingReader::new(stream, timeout);
	let res = T::read(&mut reader)?;
	Ok((res, reader.total_bytes_read()))
}

/// Read a message body from the provided stream, always blocking
/// until we have a result (or timeout).
pub fn read_body<T: Readable>(h: &MsgHeader, stream: &mut dyn Read) -> Result<T, Error> {
	let mut body = vec![0u8; h.msg_len as usize];
	read_exact(stream, &mut body, time::Duration::from_secs(20), true)?;
	ser::deserialize(&mut &body[..]).map_err(From::from)
}

/// Reads a full message from the underlying stream.
pub fn read_message<T: Readable>(stream: &mut dyn Read, msg_type: Type) -> Result<T, Error> {
	let header = read_header(stream, Some(msg_type))?;
	if header.msg_type != msg_type {
		return Err(Error::BadMessage);
	}
	read_body(&header, stream)
}

pub fn write_to_buf<T: Writeable>(msg: T, msg_type: Type) -> Vec<u8> {
	// prepare the body first so we know its serialized length
	let mut body_buf = vec![];
	ser::serialize(&mut body_buf, &msg).unwrap();

	// build and serialize the header using the body size
	let mut msg_buf = vec![];
	let blen = body_buf.len() as u64;
	ser::serialize(&mut msg_buf, &MsgHeader::new(msg_type, blen)).unwrap();
	msg_buf.append(&mut body_buf);

	msg_buf
}

pub fn write_message<T: Writeable>(
	stream: &mut dyn Write,
	msg: T,
	msg_type: Type,
) -> Result<(), Error> {
	let buf = write_to_buf(msg, msg_type);
	stream.write_all(&buf[..])?;
	Ok(())
}

/// Header of any protocol message, used to identify incoming messages.
pub struct MsgHeader {
	magic: [u8; 2],
	/// Type of the message.
	pub msg_type: Type,
	/// Total length of the message in bytes.
	pub msg_len: u64,
}

impl MsgHeader {
	/// Creates a new message header.
	pub fn new(msg_type: Type, len: u64) -> MsgHeader {
		MsgHeader {
			magic: magic(),
			msg_type: msg_type,
			msg_len: len,
		}
	}
}

impl FixedLength for MsgHeader {
	const LEN: usize = 1 + 1 + 1 + 8;
}

impl Writeable for MsgHeader {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u8, self.magic[0]],
			[write_u8, self.magic[1]],
			[write_u8, self.msg_type as u8],
			[write_u64, self.msg_len]
		);
		Ok(())
	}
}

impl Readable for MsgHeader {
	fn read(reader: &mut dyn Reader) -> Result<MsgHeader, ser::Error> {
		let m = magic();
		reader.expect_u8(m[0])?;
		reader.expect_u8(m[1])?;
		let (t, len) = ser_multiread!(reader, read_u8, read_u64);
		match Type::from_u8(t) {
			Some(ty) => Ok(MsgHeader {
				magic: m,
				msg_type: ty,
				msg_len: len,
			}),
			None => Err(ser::Error::CorruptedData),
		}
	}
}

/// First part of a handshake, sender advertises its version and
/// characteristics.
pub struct Hand {
	/// protocol version of the sender
	pub version: u32,
	/// capabilities of the sender
	pub capabilities: Capabilities,
	/// randomly generated for each handshake, helps detect self
	pub nonce: u64,
	/// genesis block of our chain, only connect to peers on the same chain
	pub genesis: Hash,
	/// total difficulty accumulated by the sender, used to check whether sync
	/// may be needed
	pub total_difficulty: Difficulty,
	/// network address of the sender
	pub sender_addr: PeerAddr,
	/// network address of the receiver
	pub receiver_addr: PeerAddr,
	/// name of version of the software
	pub user_agent: String,
}

impl Writeable for Hand {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u32, self.version],
			[write_u32, self.capabilities.bits()],
			[write_u64, self.nonce]
		);
		self.total_difficulty.write(writer).unwrap();
		self.sender_addr.write(writer).unwrap();
		self.receiver_addr.write(writer).unwrap();
		writer.write_bytes(&self.user_agent).unwrap();
		self.genesis.write(writer).unwrap();
		Ok(())
	}
}

impl Readable for Hand {
	fn read(reader: &mut dyn Reader) -> Result<Hand, ser::Error> {
		let (version, capab, nonce) = ser_multiread!(reader, read_u32, read_u32, read_u64);
		let capabilities = Capabilities::from_bits_truncate(capab);
		let total_diff = Difficulty::read(reader)?;
		let sender_addr = PeerAddr::read(reader)?;
		let receiver_addr = PeerAddr::read(reader)?;
		let ua = reader.read_bytes_len_prefix()?;
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let genesis = Hash::read(reader)?;
		Ok(Hand {
			version: version,
			capabilities: capabilities,
			nonce: nonce,
			genesis: genesis,
			total_difficulty: total_diff,
			sender_addr: sender_addr,
			receiver_addr: receiver_addr,
			user_agent: user_agent,
		})
	}
}

/// Second part of a handshake, receiver of the first part replies with its own
/// version and characteristics.
pub struct Shake {
	/// sender version
	pub version: u32,
	/// sender capabilities
	pub capabilities: Capabilities,
	/// genesis block of our chain, only connect to peers on the same chain
	pub genesis: Hash,
	/// total difficulty accumulated by the sender, used to check whether sync
	/// may be needed
	pub total_difficulty: Difficulty,
	/// name of version of the software
	pub user_agent: String,
}

impl Writeable for Shake {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(
			writer,
			[write_u32, self.version],
			[write_u32, self.capabilities.bits()]
		);
		self.total_difficulty.write(writer).unwrap();
		writer.write_bytes(&self.user_agent).unwrap();
		self.genesis.write(writer).unwrap();
		Ok(())
	}
}

impl Readable for Shake {
	fn read(reader: &mut dyn Reader) -> Result<Shake, ser::Error> {
		let (version, capab) = ser_multiread!(reader, read_u32, read_u32);
		let capabilities = Capabilities::from_bits_truncate(capab);
		let total_diff = Difficulty::read(reader)?;
		let ua = reader.read_bytes_len_prefix()?;
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let genesis = Hash::read(reader)?;
		Ok(Shake {
			version: version,
			capabilities: capabilities,
			genesis: genesis,
			total_difficulty: total_diff,
			user_agent: user_agent,
		})
	}
}

/// Ask for other peers addresses, required for network discovery.
pub struct GetPeerAddrs {
	/// Filters on the capabilities we'd like the peers to have
	pub capabilities: Capabilities,
}

impl Writeable for GetPeerAddrs {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u32(self.capabilities.bits())
	}
}

impl Readable for GetPeerAddrs {
	fn read(reader: &mut dyn Reader) -> Result<GetPeerAddrs, ser::Error> {
		let capab = reader.read_u32()?;
		let capabilities = Capabilities::from_bits_truncate(capab);
		Ok(GetPeerAddrs { capabilities })
	}
}

/// Peer addresses we know of that are fresh enough, in response to
/// GetPeerAddrs.
#[derive(Debug)]
pub struct PeerAddrs {
	pub peers: Vec<PeerAddr>,
}

impl Writeable for PeerAddrs {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u32(self.peers.len() as u32)?;
		for p in &self.peers {
			p.write(writer).unwrap();
		}
		Ok(())
	}
}

impl Readable for PeerAddrs {
	fn read(reader: &mut dyn Reader) -> Result<PeerAddrs, ser::Error> {
		let peer_count = reader.read_u32()?;
		if peer_count > MAX_PEER_ADDRS {
			return Err(ser::Error::TooLargeReadErr);
		} else if peer_count == 0 {
			return Ok(PeerAddrs { peers: vec![] });
		}
		let mut peers = Vec::with_capacity(peer_count as usize);
		for _ in 0..peer_count {
			peers.push(PeerAddr::read(reader)?);
		}
		Ok(PeerAddrs { peers: peers })
	}
}

/// We found some issue in the communication, sending an error back, usually
/// followed by closing the connection.
pub struct PeerError {
	/// error code
	pub code: u32,
	/// slightly more user friendly message
	pub message: String,
}

impl Writeable for PeerError {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		ser_multiwrite!(writer, [write_u32, self.code], [write_bytes, &self.message]);
		Ok(())
	}
}

impl Readable for PeerError {
	fn read(reader: &mut dyn Reader) -> Result<PeerError, ser::Error> {
		let code = reader.read_u32()?;
		let msg = reader.read_bytes_len_prefix()?;
		let message = String::from_utf8(msg).map_err(|_| ser::Error::CorruptedData)?;
		Ok(PeerError {
			code: code,
			message: message,
		})
	}
}

/// Serializable wrapper for the block locator.
#[derive(Debug)]
pub struct Locator {
	pub hashes: Vec<Hash>,
}

impl Writeable for Locator {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(self.hashes.len() as u8)?;
		for h in &self.hashes {
			h.write(writer)?
		}
		Ok(())
	}
}

impl Readable for Locator {
	fn read(reader: &mut dyn Reader) -> Result<Locator, ser::Error> {
		let len = reader.read_u8()?;
		if len > (MAX_LOCATORS as u8) {
			return Err(ser::Error::TooLargeReadErr);
		}
		let mut hashes = Vec::with_capacity(len as usize);
		for _ in 0..len {
			hashes.push(Hash::read(reader)?);
		}
		Ok(Locator { hashes: hashes })
	}
}

/// Serializable wrapper for a list of block headers.
pub struct Headers {
	pub headers: Vec<BlockHeader>,
}

impl Writeable for Headers {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u16(self.headers.len() as u16)?;
		for h in &self.headers {
			h.write(writer)?
		}
		Ok(())
	}
}

pub struct Ping {
	/// total difficulty accumulated by the sender, used to check whether sync
	/// may be needed
	pub total_difficulty: Difficulty,
	/// total height
	pub height: u64,
}

impl Writeable for Ping {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.total_difficulty.write(writer).unwrap();
		self.height.write(writer).unwrap();
		Ok(())
	}
}

impl Readable for Ping {
	fn read(reader: &mut dyn Reader) -> Result<Ping, ser::Error> {
		let total_difficulty = Difficulty::read(reader)?;
		let height = reader.read_u64()?;
		Ok(Ping {
			total_difficulty,
			height,
		})
	}
}

pub struct Pong {
	/// total difficulty accumulated by the sender, used to check whether sync
	/// may be needed
	pub total_difficulty: Difficulty,
	/// height accumulated by sender
	pub height: u64,
}

impl Writeable for Pong {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.total_difficulty.write(writer).unwrap();
		self.height.write(writer).unwrap();
		Ok(())
	}
}

impl Readable for Pong {
	fn read(reader: &mut dyn Reader) -> Result<Pong, ser::Error> {
		let total_difficulty = Difficulty::read(reader)?;
		let height = reader.read_u64()?;
		Ok(Pong {
			total_difficulty,
			height,
		})
	}
}

#[derive(Debug)]
pub struct BanReason {
	/// the reason for the ban
	pub ban_reason: ReasonForBan,
}

impl Writeable for BanReason {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		let ban_reason_i32 = self.ban_reason as i32;
		ban_reason_i32.write(writer).unwrap();
		Ok(())
	}
}

impl Readable for BanReason {
	fn read(reader: &mut dyn Reader) -> Result<BanReason, ser::Error> {
		let ban_reason_i32 = match reader.read_i32() {
			Ok(h) => h,
			Err(_) => 0,
		};

		let ban_reason = ReasonForBan::from_i32(ban_reason_i32).ok_or(ser::Error::CorruptedData)?;

		Ok(BanReason { ban_reason })
	}
}

/// Request to get an archive of the full txhashset store, required to sync
/// a new node.
pub struct TxHashSetRequest {
	/// Hash of the block for which the txhashset should be provided
	pub hash: Hash,
	/// Height of the corresponding block
	pub height: u64,
}

impl Writeable for TxHashSetRequest {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.hash.write(writer)?;
		writer.write_u64(self.height)?;
		Ok(())
	}
}

impl Readable for TxHashSetRequest {
	fn read(reader: &mut dyn Reader) -> Result<TxHashSetRequest, ser::Error> {
		Ok(TxHashSetRequest {
			hash: Hash::read(reader)?,
			height: reader.read_u64()?,
		})
	}
}

/// Response to a txhashset archive request, must include a zip stream of the
/// archive after the message body.
pub struct TxHashSetArchive {
	/// Hash of the block for which the txhashset are provided
	pub hash: Hash,
	/// Height of the corresponding block
	pub height: u64,
	/// Size in bytes of the archive
	pub bytes: u64,
}

impl Writeable for TxHashSetArchive {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.hash.write(writer)?;
		ser_multiwrite!(writer, [write_u64, self.height], [write_u64, self.bytes]);
		Ok(())
	}
}

impl Readable for TxHashSetArchive {
	fn read(reader: &mut dyn Reader) -> Result<TxHashSetArchive, ser::Error> {
		let hash = Hash::read(reader)?;
		let (height, bytes) = ser_multiread!(reader, read_u64, read_u64);

		Ok(TxHashSetArchive {
			hash,
			height,
			bytes,
		})
	}
}
