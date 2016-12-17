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

//! Message types that transit over the network and related serialization code.

use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6, Ipv4Addr, Ipv6Addr};
use num::FromPrimitive;

use futures::future::{Future, ok};
use tokio_core::net::TcpStream;
use tokio_core::io::{write_all, read_exact};

use core::ser::{self, Writeable, Readable, Writer, Reader};
use core::consensus::MAX_MSG_LEN;

use types::*;

/// Current latest version of the protocol
pub const PROTOCOL_VERSION: u32 = 1;
/// Grin's user agent with current version (TODO externalize)
pub const USER_AGENT: &'static str = "MW/Grin 0.1";

/// Magic number expected in the header of every message
const MAGIC: [u8; 2] = [0x1e, 0xc5];

/// Size in bytes of a message header
pub const HEADER_LEN: u64 = 11;

/// Codes for each error that can be produced reading a message.
pub enum ErrCodes {
	UnsupportedVersion = 100,
}

/// Types of messages
enum_from_primitive! {
  #[derive(Debug, Clone, Copy)]
  pub enum Type {
    Error,
    Hand,
    Shake,
    Ping,
    Pong,
    GetPeerAddrs,
    PeerAddrs,
    Block,
    Transaction,
  }
}

/// Future combinator to read any message where the body is a Readable. Reads
/// the  header first, handles its validation and then reads the Readable body,
/// allocating buffers of the right size.
pub fn read_msg<T>(conn: TcpStream) -> Box<Future<Item = (TcpStream, T), Error = ser::Error>>
	where T: Readable<T> + 'static
{
	let read_header = read_exact(conn, vec![0u8; HEADER_LEN as usize])
		.map_err(|e| ser::Error::IOErr(e))
		.and_then(|(reader, buf)| {
			let header = try!(ser::deserialize::<MsgHeader>(&mut &buf[..]));
			if header.msg_len > MAX_MSG_LEN {
				// TODO add additional restrictions on a per-message-type basis to avoid 20MB
				// pings
				return Err(ser::Error::TooLargeReadErr);
			}
			Ok((reader, header))
		});

	let read_msg = read_header.and_then(|(reader, header)| {
			read_exact(reader, vec![0u8; header.msg_len as usize]).map_err(|e| ser::Error::IOErr(e))
		})
		.and_then(|(reader, buf)| {
			let body = try!(ser::deserialize(&mut &buf[..]));
			Ok((reader, body))
		});
	Box::new(read_msg)
}

/// Future combinator to write a full message from a Writeable payload.
/// Serializes the payload first and then sends the message header and that
/// payload.
pub fn write_msg<T>(conn: TcpStream,
                    msg: T,
                    msg_type: Type)
                    -> Box<Future<Item = TcpStream, Error = ser::Error>>
	where T: Writeable + 'static
{
	let write_msg = ok((conn)).and_then(move |conn| {
		// prepare the body first so we know its serialized length
		let mut body_buf = vec![];
		ser::serialize(&mut body_buf, &msg);

		// build and serialize the header using the body size
		let mut header_buf = vec![];
		let blen = body_buf.len() as u64;
		ser::serialize(&mut header_buf, &MsgHeader::new(msg_type, blen));

		// send the whole thing
		write_all(conn, header_buf)
			.and_then(|(conn, _)| write_all(conn, body_buf))
			.map(|(conn, _)| conn)
			.map_err(|e| ser::Error::IOErr(e))
	});
	Box::new(write_msg)
}

/// Header of any protocol message, used to identify incoming messages.
pub struct MsgHeader {
	magic: [u8; 2],
	pub msg_type: Type,
	pub msg_len: u64,
}

impl MsgHeader {
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
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u8, self.magic[0]],
		                [write_u8, self.magic[1]],
		                [write_u8, self.msg_type as u8],
		                [write_u64, self.msg_len]);
		Ok(())
	}
}

impl Readable<MsgHeader> for MsgHeader {
	fn read(reader: &mut Reader) -> Result<MsgHeader, ser::Error> {
		try!(reader.expect_u8(MAGIC[0]));
		try!(reader.expect_u8(MAGIC[1]));
		let (t, len) = ser_multiread!(reader, read_u8, read_u64);
		match Type::from_u8(t) {
			Some(ty) => {
				Ok(MsgHeader {
					magic: MAGIC,
					msg_type: ty,
					msg_len: len,
				})
			}
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
	/// network address of the sender
	pub sender_addr: SockAddr,
	/// network address of the receiver
	pub receiver_addr: SockAddr,
	/// name of version of the software
	pub user_agent: String,
}

impl Writeable for Hand {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u32, self.version],
		                [write_u32, self.capabilities.bits()],
		                [write_u64, self.nonce]);
		self.sender_addr.write(writer);
		self.receiver_addr.write(writer);
		writer.write_bytes(&self.user_agent)
	}
}

impl Readable<Hand> for Hand {
	fn read(reader: &mut Reader) -> Result<Hand, ser::Error> {
		let (version, capab, nonce) = ser_multiread!(reader, read_u32, read_u32, read_u64);
		let sender_addr = try!(SockAddr::read(reader));
		let receiver_addr = try!(SockAddr::read(reader));
		let ua = try!(reader.read_vec());
		let user_agent = try!(String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData));
		let capabilities = try!(Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData));
		Ok(Hand {
			version: version,
			capabilities: capabilities,
			nonce: nonce,
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
	/// name of version of the software
	pub user_agent: String,
}

impl Writeable for Shake {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		ser_multiwrite!(writer,
		                [write_u32, self.version],
		                [write_u32, self.capabilities.bits()],
		                [write_bytes, &self.user_agent]);
		Ok(())
	}
}

impl Readable<Shake> for Shake {
	fn read(reader: &mut Reader) -> Result<Shake, ser::Error> {
		let (version, capab, ua) = ser_multiread!(reader, read_u32, read_u32, read_vec);
		let user_agent = try!(String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData));
		let capabilities = try!(Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData));
		Ok(Shake {
			version: version,
			capabilities: capabilities,
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
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		writer.write_u32(self.capabilities.bits())
	}
}

impl Readable<GetPeerAddrs> for GetPeerAddrs {
	fn read(reader: &mut Reader) -> Result<GetPeerAddrs, ser::Error> {
		let capab = try!(reader.read_u32());
		let capabilities = try!(Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData));
		Ok(GetPeerAddrs { capabilities: capabilities })
	}
}

/// Peer addresses we know of that are fresh enough, in response to
/// GetPeerAddrs.
pub struct PeerAddrs {
	pub peers: Vec<SockAddr>,
}

impl Writeable for PeerAddrs {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		try!(writer.write_u32(self.peers.len() as u32));
		for p in &self.peers {
			p.write(writer);
		}
		Ok(())
	}
}

impl Readable<PeerAddrs> for PeerAddrs {
	fn read(reader: &mut Reader) -> Result<PeerAddrs, ser::Error> {
		let peer_count = try!(reader.read_u32());
		if peer_count > 1000 {
			return Err(ser::Error::TooLargeReadErr);
		}
		let peers = try_map_vec!([0..peer_count], |_| SockAddr::read(reader));
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
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		ser_multiwrite!(writer, [write_u32, self.code], [write_bytes, &self.message]);
		Ok(())
	}
}

impl Readable<PeerError> for PeerError {
	fn read(reader: &mut Reader) -> Result<PeerError, ser::Error> {
		let (code, msg) = ser_multiread!(reader, read_u32, read_vec);
		let message = try!(String::from_utf8(msg).map_err(|_| ser::Error::CorruptedData));
		Ok(PeerError {
			code: code,
			message: message,
		})
	}
}

/// Only necessary so we can implement Readable and Writeable. Rust disallows
/// implementing traits when both types are outside of this crate (which is the
/// case for SocketAddr and Readable/Writeable).
pub struct SockAddr(pub SocketAddr);

impl Writeable for SockAddr {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		match self.0 {
			SocketAddr::V4(sav4) => {
				ser_multiwrite!(writer,
				                [write_u8, 0],
				                [write_fixed_bytes, &sav4.ip().octets().to_vec()],
				                [write_u16, sav4.port()]);
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

impl Readable<SockAddr> for SockAddr {
	fn read(reader: &mut Reader) -> Result<SockAddr, ser::Error> {
		let v4_or_v6 = try!(reader.read_u8());
		if v4_or_v6 == 0 {
			let ip = try!(reader.read_fixed_bytes(4));
			let port = try!(reader.read_u16());
			Ok(SockAddr(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(ip[0],
			                                                           ip[1],
			                                                           ip[2],
			                                                           ip[3]),
			                                             port))))
		} else {
			let ip = try_map_vec!([0..8], |_| reader.read_u16());
			let port = try!(reader.read_u16());
			Ok(SockAddr(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::new(ip[0],
			                                                           ip[1],
			                                                           ip[2],
			                                                           ip[3],
			                                                           ip[4],
			                                                           ip[5],
			                                                           ip[6],
			                                                           ip[7]),
			                                             port,
			                                             0,
			                                             0))))
		}
	}
}

/// Placeholder for messages like Ping and Pong that don't send anything but
/// the header.
pub struct Empty {}

impl Writeable for Empty {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		Ok(())
	}
}

impl Readable<Empty> for Empty {
	fn read(reader: &mut Reader) -> Result<Empty, ser::Error> {
		Ok(Empty {})
	}
}
