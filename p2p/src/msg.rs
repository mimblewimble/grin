// Copyright 2021 The Grin Developers
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

use crate::chain::txhashset::BitmapSegment;
use crate::conn::Tracker;
use crate::core::core::hash::Hash;
use crate::core::core::transaction::{OutputIdentifier, TxKernel};
use crate::core::core::{
	BlockHeader, Segment, SegmentIdentifier, Transaction, UntrustedBlock, UntrustedBlockHeader,
	UntrustedCompactBlock,
};
use crate::core::pow::Difficulty;
use crate::core::ser::{
	self, DeserializationMode, ProtocolVersion, Readable, Reader, StreamingReader, Writeable,
	Writer,
};
use crate::core::{consensus, global};
use crate::types::{
	AttachmentMeta, AttachmentUpdate, Capabilities, Error, PeerAddr, ReasonForBan,
	MAX_BLOCK_HEADERS, MAX_LOCATORS, MAX_PEER_ADDRS,
};
use crate::util::secp::pedersen::RangeProof;
use bytes::Bytes;
use num::FromPrimitive;
use std::fs::File;
use std::io::{Read, Write};
use std::sync::Arc;
use std::{fmt, thread, time::Duration};

/// Grin's user agent with current version
pub const USER_AGENT: &str = concat!("MW/Grin ", env!("CARGO_PKG_VERSION"));

/// Magic numbers expected in the header of every message
const OTHER_MAGIC: [u8; 2] = [73, 43];
const TESTNET_MAGIC: [u8; 2] = [83, 59];
const MAINNET_MAGIC: [u8; 2] = [97, 61];

// Types of messages.
// Note: Values here are *important* so we should only add new values at the
// end.
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
		GetOutputBitmapSegment = 21,
		OutputBitmapSegment = 22,
		GetOutputSegment = 23,
		OutputSegment = 24,
		GetRangeProofSegment = 25,
		RangeProofSegment = 26,
		GetKernelSegment = 27,
		KernelSegment = 28,
	}
}

/// Max theoretical size of a block filled with outputs.
fn max_block_size() -> u64 {
	(global::max_block_weight() / consensus::OUTPUT_WEIGHT * 708) as u64
}

// Max msg size when msg type is unknown.
fn default_max_msg_size() -> u64 {
	max_block_size()
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
		Type::GetOutputBitmapSegment => 41,
		Type::OutputBitmapSegment => 2 * max_block_size(),
		Type::GetOutputSegment => 41,
		Type::OutputSegment => 2 * max_block_size(),
		Type::GetRangeProofSegment => 41,
		Type::RangeProofSegment => 2 * max_block_size(),
		Type::GetKernelSegment => 41,
		Type::KernelSegment => 2 * max_block_size(),
	}
}

fn magic() -> [u8; 2] {
	match global::get_chain_type() {
		global::ChainTypes::Testnet => TESTNET_MAGIC,
		global::ChainTypes::Mainnet => MAINNET_MAGIC,
		_ => OTHER_MAGIC,
	}
}

pub struct Msg {
	header: MsgHeader,
	body: Vec<u8>,
	attachment: Option<File>,
	version: ProtocolVersion,
}

impl Msg {
	pub fn new<T: Writeable>(
		msg_type: Type,
		msg: T,
		version: ProtocolVersion,
	) -> Result<Msg, Error> {
		let body = ser::ser_vec(&msg, version)?;
		Ok(Msg {
			header: MsgHeader::new(msg_type, body.len() as u64),
			body,
			attachment: None,
			version,
		})
	}

	pub fn add_attachment(&mut self, attachment: File) {
		self.attachment = Some(attachment)
	}
}

/// Read a header from the provided stream without blocking if the
/// underlying stream is async. Typically headers will be polled for, so
/// we do not want to block.
///
/// Note: We return a MsgHeaderWrapper here as we may encounter an unknown msg type.
///
pub fn read_header<R: Read>(
	stream: &mut R,
	version: ProtocolVersion,
) -> Result<MsgHeaderWrapper, Error> {
	let mut head = vec![0u8; MsgHeader::LEN];
	stream.read_exact(&mut head)?;
	let header: MsgHeaderWrapper =
		ser::deserialize(&mut &head[..], version, DeserializationMode::default())?;
	Ok(header)
}

/// Read a single item from the provided stream, always blocking until we
/// have a result (or timeout).
/// Returns the item and the total bytes read.
pub fn read_item<T: Readable, R: Read>(
	stream: &mut R,
	version: ProtocolVersion,
) -> Result<(T, u64), Error> {
	let mut reader = StreamingReader::new(stream, version);
	let res = T::read(&mut reader)?;
	Ok((res, reader.total_bytes_read()))
}

/// Read a message body from the provided stream, always blocking
/// until we have a result (or timeout).
pub fn read_body<T: Readable, R: Read>(
	h: &MsgHeader,
	stream: &mut R,
	version: ProtocolVersion,
) -> Result<T, Error> {
	let mut body = vec![0u8; h.msg_len as usize];
	stream.read_exact(&mut body)?;
	ser::deserialize(&mut &body[..], version, DeserializationMode::default()).map_err(From::from)
}

/// Read (an unknown) message from the provided stream and discard it.
pub fn read_discard<R: Read>(msg_len: u64, stream: &mut R) -> Result<(), Error> {
	let mut buffer = vec![0u8; msg_len as usize];
	stream.read_exact(&mut buffer)?;
	Ok(())
}

/// Reads a full message from the underlying stream.
pub fn read_message<T: Readable, R: Read>(
	stream: &mut R,
	version: ProtocolVersion,
	msg_type: Type,
) -> Result<T, Error> {
	match read_header(stream, version)? {
		MsgHeaderWrapper::Known(header) => {
			if header.msg_type == msg_type {
				read_body(&header, stream, version)
			} else {
				Err(Error::BadMessage)
			}
		}
		MsgHeaderWrapper::Unknown(msg_len, _) => {
			read_discard(msg_len, stream)?;
			Err(Error::BadMessage)
		}
	}
}

pub fn write_message<W: Write>(
	stream: &mut W,
	msg: &Msg,
	tracker: Arc<Tracker>,
) -> Result<(), Error> {
	// Introduce a delay so messages are spaced at least 150ms apart.
	// This gives a max msg rate of 60000/150 = 400 messages per minute.
	// Exceeding 500 messages per minute will result in being banned as abusive.
	if let Some(elapsed) = tracker.sent_bytes.read().elapsed_since_last_msg() {
		let min_interval: u64 = 150;
		let sleep_ms = min_interval.saturating_sub(elapsed);
		if sleep_ms > 0 {
			thread::sleep(Duration::from_millis(sleep_ms))
		}
	}

	let mut buf = ser::ser_vec(&msg.header, msg.version)?;
	buf.extend(&msg.body[..]);
	stream.write_all(&buf[..])?;
	tracker.inc_sent(buf.len() as u64);
	if let Some(file) = &msg.attachment {
		let mut file = file.try_clone()?;
		let mut buf = [0u8; 8000];
		loop {
			match file.read(&mut buf[..]) {
				Ok(0) => break,
				Ok(n) => {
					stream.write_all(&buf[..n])?;
					// Increase sent bytes "quietly" without incrementing the counter.
					// (In a loop here for the single attachment).
					tracker.inc_quiet_sent(n as u64);
				}
				Err(e) => return Err(From::from(e)),
			}
		}
	}
	Ok(())
}

/// A wrapper around a message header. If the header is for an unknown msg type
/// then we will be unable to parse the msg itself (just a bunch of random bytes).
/// But we need to know how many bytes to discard to discard the full message.
#[derive(Clone)]
pub enum MsgHeaderWrapper {
	/// A "known" msg type with deserialized msg header.
	Known(MsgHeader),
	/// An unknown msg type with corresponding msg size in bytes.
	Unknown(u64, u8),
}

/// Header of any protocol message, used to identify incoming messages.
#[derive(Clone)]
pub struct MsgHeader {
	magic: [u8; 2],
	/// Type of the message.
	pub msg_type: Type,
	/// Total length of the message in bytes.
	pub msg_len: u64,
}

impl MsgHeader {
	// 2 magic bytes + 1 type byte + 8 bytes (msg_len)
	pub const LEN: usize = 2 + 1 + 8;

	/// Creates a new message header.
	pub fn new(msg_type: Type, len: u64) -> MsgHeader {
		MsgHeader {
			magic: magic(),
			msg_type: msg_type,
			msg_len: len,
		}
	}
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

impl Readable for MsgHeaderWrapper {
	fn read<R: Reader>(reader: &mut R) -> Result<MsgHeaderWrapper, ser::Error> {
		let m = magic();
		reader.expect_u8(m[0])?;
		reader.expect_u8(m[1])?;

		// Read the msg header.
		// We do not yet know if the msg type is one we support locally.
		let (t, msg_len) = ser_multiread!(reader, read_u8, read_u64);

		// Attempt to convert the msg type byte into one of our known msg type enum variants.
		// Check the msg_len while we are at it.
		match Type::from_u8(t) {
			Some(msg_type) => {
				// TODO 4x the limits for now to leave ourselves space to change things.
				let max_len = max_msg_size(msg_type) * 4;
				if msg_len > max_len {
					error!(
						"Too large read {:?}, max_len: {}, msg_len: {}.",
						msg_type, max_len, msg_len
					);
					return Err(ser::Error::TooLargeReadErr);
				}

				Ok(MsgHeaderWrapper::Known(MsgHeader {
					magic: m,
					msg_type,
					msg_len,
				}))
			}
			None => {
				// Unknown msg type, but we still want to limit how big the msg is.
				let max_len = default_max_msg_size() * 4;
				if msg_len > max_len {
					error!(
						"Too large read (unknown msg type) {:?}, max_len: {}, msg_len: {}.",
						t, max_len, msg_len
					);
					return Err(ser::Error::TooLargeReadErr);
				}

				Ok(MsgHeaderWrapper::Unknown(msg_len, t))
			}
		}
	}
}

/// First part of a handshake, sender advertises its version and
/// characteristics.
pub struct Hand {
	/// protocol version of the sender
	pub version: ProtocolVersion,
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
		self.version.write(writer)?;
		ser_multiwrite!(
			writer,
			[write_u32, self.capabilities.bits()],
			[write_u64, self.nonce]
		);
		self.total_difficulty.write(writer)?;
		self.sender_addr.write(writer)?;
		self.receiver_addr.write(writer)?;
		writer.write_bytes(&self.user_agent)?;
		self.genesis.write(writer)?;
		Ok(())
	}
}

impl Readable for Hand {
	fn read<R: Reader>(reader: &mut R) -> Result<Hand, ser::Error> {
		let version = ProtocolVersion::read(reader)?;
		let (capab, nonce) = ser_multiread!(reader, read_u32, read_u64);
		let capabilities = Capabilities::from_bits_truncate(capab);
		let total_difficulty = Difficulty::read(reader)?;
		let sender_addr = PeerAddr::read(reader)?;
		let receiver_addr = PeerAddr::read(reader)?;
		let ua = reader.read_bytes_len_prefix()?;
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let genesis = Hash::read(reader)?;
		Ok(Hand {
			version,
			capabilities,
			nonce,
			genesis,
			total_difficulty,
			sender_addr,
			receiver_addr,
			user_agent,
		})
	}
}

/// Second part of a handshake, receiver of the first part replies with its own
/// version and characteristics.
pub struct Shake {
	/// sender version
	pub version: ProtocolVersion,
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
		self.version.write(writer)?;
		writer.write_u32(self.capabilities.bits())?;
		self.total_difficulty.write(writer)?;
		writer.write_bytes(&self.user_agent)?;
		self.genesis.write(writer)?;
		Ok(())
	}
}

impl Readable for Shake {
	fn read<R: Reader>(reader: &mut R) -> Result<Shake, ser::Error> {
		let version = ProtocolVersion::read(reader)?;
		let capab = reader.read_u32()?;
		let capabilities = Capabilities::from_bits_truncate(capab);
		let total_difficulty = Difficulty::read(reader)?;
		let ua = reader.read_bytes_len_prefix()?;
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let genesis = Hash::read(reader)?;
		Ok(Shake {
			version,
			capabilities,
			genesis,
			total_difficulty,
			user_agent,
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
	fn read<R: Reader>(reader: &mut R) -> Result<GetPeerAddrs, ser::Error> {
		let capab = reader.read_u32()?;
		let capabilities = Capabilities::from_bits_truncate(capab);
		Ok(GetPeerAddrs { capabilities })
	}
}

/// Peer addresses we know of that are fresh enough, in response to
/// GetPeerAddrs.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PeerAddrs {
	pub peers: Vec<PeerAddr>,
}

impl Writeable for PeerAddrs {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u32(self.peers.len() as u32)?;
		for p in &self.peers {
			p.write(writer)?;
		}
		Ok(())
	}
}

impl Readable for PeerAddrs {
	fn read<R: Reader>(reader: &mut R) -> Result<PeerAddrs, ser::Error> {
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
		Ok(PeerAddrs { peers })
	}
}

impl IntoIterator for PeerAddrs {
	type Item = PeerAddr;
	type IntoIter = std::vec::IntoIter<Self::Item>;
	fn into_iter(self) -> Self::IntoIter {
		self.peers.into_iter()
	}
}

impl Default for PeerAddrs {
	fn default() -> Self {
		PeerAddrs { peers: vec![] }
	}
}

impl PeerAddrs {
	pub fn as_slice(&self) -> &[PeerAddr] {
		self.peers.as_slice()
	}

	pub fn contains(&self, addr: &PeerAddr) -> bool {
		self.peers.contains(addr)
	}

	pub fn difference(&self, other: &[PeerAddr]) -> PeerAddrs {
		let peers = self
			.peers
			.iter()
			.filter(|x| !other.contains(x))
			.cloned()
			.collect();
		PeerAddrs { peers }
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
	fn read<R: Reader>(reader: &mut R) -> Result<PeerError, ser::Error> {
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
	fn read<R: Reader>(reader: &mut R) -> Result<Locator, ser::Error> {
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
		self.total_difficulty.write(writer)?;
		self.height.write(writer)?;
		Ok(())
	}
}

impl Readable for Ping {
	fn read<R: Reader>(reader: &mut R) -> Result<Ping, ser::Error> {
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
		self.total_difficulty.write(writer)?;
		self.height.write(writer)?;
		Ok(())
	}
}

impl Readable for Pong {
	fn read<R: Reader>(reader: &mut R) -> Result<Pong, ser::Error> {
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
		ban_reason_i32.write(writer)?;
		Ok(())
	}
}

impl Readable for BanReason {
	fn read<R: Reader>(reader: &mut R) -> Result<BanReason, ser::Error> {
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
	fn read<R: Reader>(reader: &mut R) -> Result<TxHashSetRequest, ser::Error> {
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
	fn read<R: Reader>(reader: &mut R) -> Result<TxHashSetArchive, ser::Error> {
		let hash = Hash::read(reader)?;
		let (height, bytes) = ser_multiread!(reader, read_u64, read_u64);

		Ok(TxHashSetArchive {
			hash,
			height,
			bytes,
		})
	}
}

/// Request to get a segment of a (P)MMR at a particular block.
pub struct SegmentRequest {
	/// The hash of the block the MMR is associated with
	pub block_hash: Hash,
	/// The identifier of the requested segment
	pub identifier: SegmentIdentifier,
}

impl Readable for SegmentRequest {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		let block_hash = Readable::read(reader)?;
		let identifier = Readable::read(reader)?;
		Ok(Self {
			block_hash,
			identifier,
		})
	}
}

impl Writeable for SegmentRequest {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		Writeable::write(&self.block_hash, writer)?;
		Writeable::write(&self.identifier, writer)
	}
}

/// Response to a (P)MMR segment request.
pub struct SegmentResponse<T> {
	/// The hash of the block the MMR is associated with
	pub block_hash: Hash,
	/// The MMR segment
	pub segment: Segment<T>,
}

impl<T: Readable> Readable for SegmentResponse<T> {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		let block_hash = Readable::read(reader)?;
		let segment = Readable::read(reader)?;
		Ok(Self {
			block_hash,
			segment,
		})
	}
}

impl<T: Writeable> Writeable for SegmentResponse<T> {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		Writeable::write(&self.block_hash, writer)?;
		Writeable::write(&self.segment, writer)
	}
}

/// Response to an output PMMR segment request.
pub struct OutputSegmentResponse {
	/// The segment response
	pub response: SegmentResponse<OutputIdentifier>,
	/// The root hash of the output bitmap MMR
	pub output_bitmap_root: Hash,
}

impl Readable for OutputSegmentResponse {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		let response = Readable::read(reader)?;
		let output_bitmap_root = Readable::read(reader)?;
		Ok(Self {
			response,
			output_bitmap_root,
		})
	}
}

impl Writeable for OutputSegmentResponse {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		Writeable::write(&self.response, writer)?;
		Writeable::write(&self.output_bitmap_root, writer)
	}
}

/// Response to an output bitmap MMR segment request.
pub struct OutputBitmapSegmentResponse {
	/// The hash of the block the MMR is associated with
	pub block_hash: Hash,
	/// The MMR segment
	pub segment: BitmapSegment,
	/// The root hash of the output PMMR
	pub output_root: Hash,
}

impl Readable for OutputBitmapSegmentResponse {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		let block_hash = Readable::read(reader)?;
		let segment = Readable::read(reader)?;
		let output_root = Readable::read(reader)?;
		Ok(Self {
			block_hash,
			segment,
			output_root,
		})
	}
}

impl Writeable for OutputBitmapSegmentResponse {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		Writeable::write(&self.block_hash, writer)?;
		Writeable::write(&self.segment, writer)?;
		Writeable::write(&self.output_root, writer)
	}
}

pub enum Message {
	Unknown(u8),
	Ping(Ping),
	Pong(Pong),
	BanReason(BanReason),
	TransactionKernel(Hash),
	GetTransaction(Hash),
	Transaction(Transaction),
	StemTransaction(Transaction),
	GetBlock(Hash),
	Block(UntrustedBlock),
	GetCompactBlock(Hash),
	CompactBlock(UntrustedCompactBlock),
	GetHeaders(Locator),
	Header(UntrustedBlockHeader),
	Headers(HeadersData),
	GetPeerAddrs(GetPeerAddrs),
	PeerAddrs(PeerAddrs),
	TxHashSetRequest(TxHashSetRequest),
	TxHashSetArchive(TxHashSetArchive),
	Attachment(AttachmentUpdate, Option<Bytes>),
	GetOutputBitmapSegment(SegmentRequest),
	OutputBitmapSegment(OutputBitmapSegmentResponse),
	GetOutputSegment(SegmentRequest),
	OutputSegment(OutputSegmentResponse),
	GetRangeProofSegment(SegmentRequest),
	RangeProofSegment(SegmentResponse<RangeProof>),
	GetKernelSegment(SegmentRequest),
	KernelSegment(SegmentResponse<TxKernel>),
}

/// We receive 512 headers from a peer.
/// But we process them in smaller batches of 32 headers.
/// HeadersData wraps the current batch and a count of the headers remaining after this batch.
pub struct HeadersData {
	/// Batch of headers currently being processed.
	pub headers: Vec<BlockHeader>,
	/// Number of headers stil to be processed after this current batch.
	/// 0 indicates this is the final batch from the larger set of headers received from the peer.
	pub remaining: u64,
}

impl fmt::Display for Message {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Message::Unknown(_) => write!(f, "unknown"),
			Message::Ping(_) => write!(f, "ping"),
			Message::Pong(_) => write!(f, "pong"),
			Message::BanReason(_) => write!(f, "ban reason"),
			Message::TransactionKernel(_) => write!(f, "tx kernel"),
			Message::GetTransaction(_) => write!(f, "get tx"),
			Message::Transaction(_) => write!(f, "tx"),
			Message::StemTransaction(_) => write!(f, "stem tx"),
			Message::GetBlock(_) => write!(f, "get block"),
			Message::Block(_) => write!(f, "block"),
			Message::GetCompactBlock(_) => write!(f, "get compact block"),
			Message::CompactBlock(_) => write!(f, "compact block"),
			Message::GetHeaders(_) => write!(f, "get headers"),
			Message::Header(_) => write!(f, "header"),
			Message::Headers(_) => write!(f, "headers"),
			Message::GetPeerAddrs(_) => write!(f, "get peer addrs"),
			Message::PeerAddrs(_) => write!(f, "peer addrs"),
			Message::TxHashSetRequest(_) => write!(f, "tx hash set request"),
			Message::TxHashSetArchive(_) => write!(f, "tx hash set"),
			Message::Attachment(_, _) => write!(f, "attachment"),
			Message::GetOutputBitmapSegment(_) => write!(f, "get output bitmap segment"),
			Message::OutputBitmapSegment(_) => write!(f, "output bitmap segment"),
			Message::GetOutputSegment(_) => write!(f, "get output segment"),
			Message::OutputSegment(_) => write!(f, "output segment"),
			Message::GetRangeProofSegment(_) => write!(f, "get range proof segment"),
			Message::RangeProofSegment(_) => write!(f, "range proof segment"),
			Message::GetKernelSegment(_) => write!(f, "get kernel segment"),
			Message::KernelSegment(_) => write!(f, "kernel segment"),
		}
	}
}

impl fmt::Debug for Message {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Consume({})", self)
	}
}

pub enum Consumed {
	Response(Msg),
	Attachment(Arc<AttachmentMeta>, File),
	None,
	Disconnect,
}

impl fmt::Debug for Consumed {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Consumed::Response(msg) => write!(f, "Consumed::Response({:?})", msg.header.msg_type),
			Consumed::Attachment(meta, _) => write!(f, "Consumed::Attachment({:?})", meta.size),
			Consumed::None => write!(f, "Consumed::None"),
			Consumed::Disconnect => write!(f, "Consumed::Disconnect"),
		}
	}
}
