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
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, TcpStream};
use std::thread;
use std::time;

use core::consensus::{MAX_MSG_LEN, MAX_TX_INPUTS, MAX_TX_KERNELS, MAX_TX_OUTPUTS};
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::BlockHeader;
use core::ser::{self, Readable, Reader, Writeable, Writer};

use types::*;
use util::LOGGER;

/// Current latest version of the protocol
pub const PROTOCOL_VERSION: u32 = 1;

/// Grin's user agent with current version
pub const USER_AGENT: &'static str = concat!("MW/Grin ", env!("CARGO_PKG_VERSION"));

/// Magic number expected in the header of every message
const MAGIC: [u8; 2] = [0x1e, 0xc5];

/// Size in bytes of a message header
pub const HEADER_LEN: u64 = 11;

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
	}
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
		Type::Block => MAX_MSG_LEN,
		Type::GetCompactBlock => 32,
		Type::CompactBlock => MAX_MSG_LEN / 10,
		Type::StemTransaction => 1000 * MAX_TX_INPUTS + 710 * MAX_TX_OUTPUTS + 114 * MAX_TX_KERNELS,
		Type::Transaction => 1000 * MAX_TX_INPUTS + 710 * MAX_TX_OUTPUTS + 114 * MAX_TX_KERNELS,
		Type::TxHashSetRequest => 40,
		Type::TxHashSetArchive => 64,
		Type::BanReason => 64,
	}
}

/// The default implementation of read_exact is useless with async TcpStream as
/// it will return as soon as something has been read, regardless of
/// whether the buffer has been filled (and then errors). This implementation
/// will block until it has read exactly `len` bytes and returns them as a
/// `vec<u8>`. Except for a timeout, this implementation will never return a
/// partially filled buffer.
///
/// The timeout in milliseconds aborts the read when it's met. Note that the
/// time is not guaranteed to be exact. To support cases where we want to poll
/// instead of blocking, a `block_on_empty` boolean, when false, ensures
/// `read_exact` returns early with a `io::ErrorKind::WouldBlock` if nothing
/// has been read from the socket.
pub fn read_exact(
	conn: &mut TcpStream,
	mut buf: &mut [u8],
	timeout: u32,
	block_on_empty: bool,
) -> io::Result<()> {
	let sleep_time = time::Duration::from_millis(1);
	let mut count = 0;

	let mut read = 0;
	loop {
		match conn.read(buf) {
			Ok(0) => break,
			Ok(n) => {
				let tmp = buf;
				buf = &mut tmp[n..];
				read += n;
			}
			Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
			Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
				if read == 0 && !block_on_empty {
					return Err(io::Error::new(io::ErrorKind::WouldBlock, "read_exact"));
				}
			}
			Err(e) => return Err(e),
		}
		if !buf.is_empty() {
			thread::sleep(sleep_time);
			count += 1;
		} else {
			break;
		}
		if count > timeout {
			return Err(io::Error::new(
				io::ErrorKind::TimedOut,
				"reading from tcp stream",
			));
		}
	}
	Ok(())
}

/// Same as `read_exact` but for writing.
pub fn write_all(conn: &mut Write, mut buf: &[u8], timeout: u32) -> io::Result<()> {
	let sleep_time = time::Duration::from_millis(1);
	let mut count = 0;

	while !buf.is_empty() {
		match conn.write(buf) {
			Ok(0) => {
				return Err(io::Error::new(
					io::ErrorKind::WriteZero,
					"failed to write whole buffer",
				))
			}
			Ok(n) => buf = &buf[n..],
			Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
			Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
			Err(e) => return Err(e),
		}
		if !buf.is_empty() {
			thread::sleep(sleep_time);
			count += 1;
		} else {
			break;
		}
		if count > timeout {
			return Err(io::Error::new(
				io::ErrorKind::TimedOut,
				"reading from tcp stream",
			));
		}
	}
	Ok(())
}

/// Read a header from the provided connection without blocking if the
/// underlying stream is async. Typically headers will be polled for, so
/// we do not want to block.
pub fn read_header(conn: &mut TcpStream) -> Result<MsgHeader, Error> {
	let mut head = vec![0u8; HEADER_LEN as usize];
	read_exact(conn, &mut head, 10000, false)?;
	let header = ser::deserialize::<MsgHeader>(&mut &head[..])?;
	let max_len = max_msg_size(header.msg_type);
	// TODO 4x the limits for now to leave ourselves space to change things
	if header.msg_len > max_len * 4 {
		error!(
			LOGGER,
			"Too large read {}, had {}, wanted {}.", header.msg_type as u8, max_len, header.msg_len
		);
		return Err(Error::Serialization(ser::Error::TooLargeReadErr));
	}
	Ok(header)
}

/// Read a message body from the provided connection, always blocking
/// until we have a result (or timeout).
pub fn read_body<T>(h: &MsgHeader, conn: &mut TcpStream) -> Result<T, Error>
where
	T: Readable,
{
	let mut body = vec![0u8; h.msg_len as usize];
	read_exact(conn, &mut body, 20000, true)?;
	ser::deserialize(&mut &body[..]).map_err(From::from)
}

/// Reads a full message from the underlying connection.
pub fn read_message<T>(conn: &mut TcpStream, msg_type: Type) -> Result<T, Error>
where
	T: Readable,
{
	let header = read_header(conn)?;
	if header.msg_type != msg_type {
		return Err(Error::BadMessage);
	}
	read_body(&header, conn)
}

pub fn write_to_buf<T>(msg: T, msg_type: Type) -> Vec<u8>
where
	T: Writeable,
{
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

pub fn write_message<T>(conn: &mut TcpStream, msg: T, msg_type: Type) -> Result<(), Error>
where
	T: Writeable + 'static,
{
	let buf = write_to_buf(msg, msg_type);
	// send the whole thing
	conn.write_all(&buf[..])?;
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
			magic: MAGIC,
			msg_type: msg_type,
			msg_len: len,
		}
	}

	/// Serialized length of the header in bytes
	pub fn serialized_len(&self) -> u64 {
		HEADER_LEN
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

impl Readable for MsgHeader {
	fn read(reader: &mut Reader) -> Result<MsgHeader, ser::Error> {
		try!(reader.expect_u8(MAGIC[0]));
		try!(reader.expect_u8(MAGIC[1]));
		let (t, len) = ser_multiread!(reader, read_u8, read_u64);
		match Type::from_u8(t) {
			Some(ty) => Ok(MsgHeader {
				magic: MAGIC,
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
	pub sender_addr: SockAddr,
	/// network address of the receiver
	pub receiver_addr: SockAddr,
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
	fn read(reader: &mut Reader) -> Result<Hand, ser::Error> {
		let (version, capab, nonce) = ser_multiread!(reader, read_u32, read_u32, read_u64);
		let capabilities = try!(Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData,));
		let total_diff = try!(Difficulty::read(reader));
		let sender_addr = try!(SockAddr::read(reader));
		let receiver_addr = try!(SockAddr::read(reader));
		let ua = try!(reader.read_vec());
		let user_agent = try!(String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData));
		let genesis = try!(Hash::read(reader));
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
	fn read(reader: &mut Reader) -> Result<Shake, ser::Error> {
		let (version, capab) = ser_multiread!(reader, read_u32, read_u32);
		let capabilities = try!(Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData,));
		let total_diff = try!(Difficulty::read(reader));
		let ua = try!(reader.read_vec());
		let user_agent = try!(String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData));
		let genesis = try!(Hash::read(reader));
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
	fn read(reader: &mut Reader) -> Result<GetPeerAddrs, ser::Error> {
		let capab = try!(reader.read_u32());
		let capabilities = try!(Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData,));
		Ok(GetPeerAddrs {
			capabilities: capabilities,
		})
	}
}

/// Peer addresses we know of that are fresh enough, in response to
/// GetPeerAddrs.
#[derive(Debug)]
pub struct PeerAddrs {
	pub peers: Vec<SockAddr>,
}

impl Writeable for PeerAddrs {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		try!(writer.write_u32(self.peers.len() as u32));
		for p in &self.peers {
			p.write(writer).unwrap();
		}
		Ok(())
	}
}

impl Readable for PeerAddrs {
	fn read(reader: &mut Reader) -> Result<PeerAddrs, ser::Error> {
		let peer_count = try!(reader.read_u32());
		if peer_count > MAX_PEER_ADDRS {
			return Err(ser::Error::TooLargeReadErr);
		} else if peer_count == 0 {
			return Ok(PeerAddrs { peers: vec![] });
		}
		// let peers = try_map_vec!([0..peer_count], |_| SockAddr::read(reader));
		let mut peers = Vec::with_capacity(peer_count as usize);
		for _ in 0..peer_count {
			peers.push(SockAddr::read(reader)?);
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
	fn read(reader: &mut Reader) -> Result<PeerError, ser::Error> {
		let (code, msg) = ser_multiread!(reader, read_u32, read_vec);
		let message = try!(String::from_utf8(msg).map_err(|_| ser::Error::CorruptedData,));
		Ok(PeerError {
			code: code,
			message: message,
		})
	}
}

/// Only necessary so we can implement Readable and Writeable. Rust disallows
/// implementing traits when both types are outside of this crate (which is the
/// case for SocketAddr and Readable/Writeable).
#[derive(Debug)]
pub struct SockAddr(pub SocketAddr);

impl Writeable for SockAddr {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		match self.0 {
			SocketAddr::V4(sav4) => {
				ser_multiwrite!(
					writer,
					[write_u8, 0],
					[write_fixed_bytes, &sav4.ip().octets().to_vec()],
					[write_u16, sav4.port()]
				);
			}
			SocketAddr::V6(sav6) => {
				try!(writer.write_u8(1));
				for seg in &sav6.ip().segments() {
					try!(writer.write_u16(*seg));
				}
				try!(writer.write_u16(sav6.port()));
			}
		}
		Ok(())
	}
}

impl Readable for SockAddr {
	fn read(reader: &mut Reader) -> Result<SockAddr, ser::Error> {
		let v4_or_v6 = try!(reader.read_u8());
		if v4_or_v6 == 0 {
			let ip = try!(reader.read_fixed_bytes(4));
			let port = try!(reader.read_u16());
			Ok(SockAddr(SocketAddr::V4(SocketAddrV4::new(
				Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
				port,
			))))
		} else {
			let ip = try_map_vec!([0..8], |_| reader.read_u16());
			let port = try!(reader.read_u16());
			Ok(SockAddr(SocketAddr::V6(SocketAddrV6::new(
				Ipv6Addr::new(ip[0], ip[1], ip[2], ip[3], ip[4], ip[5], ip[6], ip[7]),
				port,
				0,
				0,
			))))
		}
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
	fn read(reader: &mut Reader) -> Result<Locator, ser::Error> {
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

impl Readable for Headers {
	fn read(reader: &mut Reader) -> Result<Headers, ser::Error> {
		let len = reader.read_u16()?;
		if (len as u32) > MAX_BLOCK_HEADERS + 1 {
			return Err(ser::Error::TooLargeReadErr);
		}
		let mut headers = Vec::with_capacity(len as usize);
		for _ in 0..len {
			headers.push(BlockHeader::read(reader)?);
		}
		Ok(Headers { headers: headers })
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
	fn read(reader: &mut Reader) -> Result<Ping, ser::Error> {
		// TODO - once everyone is sending total_difficulty we can clean this up
		let total_difficulty = match Difficulty::read(reader) {
			Ok(diff) => diff,
			Err(_) => Difficulty::zero(),
		};
		let height = match reader.read_u64() {
			Ok(h) => h,
			Err(_) => 0,
		};
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
	fn read(reader: &mut Reader) -> Result<Pong, ser::Error> {
		// TODO - once everyone is sending total_difficulty we can clean this up
		let total_difficulty = match Difficulty::read(reader) {
			Ok(diff) => diff,
			Err(_) => Difficulty::zero(),
		};
		let height = match reader.read_u64() {
			Ok(h) => h,
			Err(_) => 0,
		};
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
	fn read(reader: &mut Reader) -> Result<BanReason, ser::Error> {
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
	fn read(reader: &mut Reader) -> Result<TxHashSetRequest, ser::Error> {
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
	/// Output tree index the receiver should rewind to
	pub rewind_to_output: u64,
	/// Kernel tree index the receiver should rewind to
	pub rewind_to_kernel: u64,
	/// Size in bytes of the archive
	pub bytes: u64,
}

impl Writeable for TxHashSetArchive {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.hash.write(writer)?;
		ser_multiwrite!(
			writer,
			[write_u64, self.height],
			[write_u64, self.rewind_to_output],
			[write_u64, self.rewind_to_kernel],
			[write_u64, self.bytes]
		);
		Ok(())
	}
}

impl Readable for TxHashSetArchive {
	fn read(reader: &mut Reader) -> Result<TxHashSetArchive, ser::Error> {
		let hash = Hash::read(reader)?;
		let (height, rewind_to_output, rewind_to_kernel, bytes) =
			ser_multiread!(reader, read_u64, read_u64, read_u64, read_u64);

		Ok(TxHashSetArchive {
			hash,
			height,
			rewind_to_output,
			rewind_to_kernel,
			bytes,
		})
	}
}
