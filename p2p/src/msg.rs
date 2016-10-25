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

use std::net::SocketAddr;

use core::ser::{Writeable, Readable, Writer, Reader, Error};

/// Magic number expected in the header of every message
const MAGIC: [u8; 2] = [0x1e, 0xc5];

/// Codes for each error that can be produced reading a message.
enum ErrCodes {
	UNSUPPORTED_VERSION = 100,
}

bitflags! {
  /// Options for block validation
  pub flags Capabilities: u32 {
    /// We don't know (yet) what the peer can do.
    const UNKNOWN = 0b00000000,
    /// Runs with the easier version of the Proof of Work, mostly to make testing easier.
    const FULL_SYNC = 0b00000001,
  }
}

/// Types of messages
enum Type {
	HAND = 1,
	SHAKE = 2,
	ERROR = 3,
	/// Never actually used over the network but used to detect unrecognized
	/// types.
	/// Increment as needed.
	MAX_MSG_TYPE = 4,
}

/// Header of any protocol message, used to identify incoming messages.
pub struct MsgHeader {
	magic: [u8; 2],
	msg_type: Type,
}

impl MsgHeader {
	fn acceptable(&self) -> bool {
		msg_type < MAX_MSG_TYPE;
	}
}

impl Writeable for MsgHeader {
	fn write(&self, writer: &mut Writer) -> Option<ser::Error> {
		ser_multiwrite!(writer,
		                [write_u8, self.magic[0]],
		                [write_u8, self.magic[1]],
		                [write_u8, self.msg_type as u8]);
		None
	}
}

impl Readable<MsgHeader> for MsgHeader {
	fn read(reader: &mut Reader) -> Result<MsgHeader, ser::Error> {
		try!(reader.expect_u8(MAGIC[0]));
		try!(reader.expect_u8(MAGIC[1]));
		let t = try!(reader.read_u8());
		Ok(MsgHeader {
			magic: MAGIC,
			msg_type: t,
		})
	}
}

/// First part of a handshake, sender advertises its version and
/// characteristics.
pub struct Hand {
	version: u32,
	capabilities: Capabilities,
	sender_addr: SocketAddr,
	receiver_addr: SocketAddr,
	user_agent: String,
}

impl Writeable for Hand {
	fn write(&self, writer: &mut Writer) -> Option<ser::Error> {
		ser_multiwrite!(writer,
		                [write_u32, self.version],
		                [write_u32, self.capabilities]);
		sender_addr.write(writer);
		receiver_addr.write(writer);
		writer.write_vec(&mut self.user_agent.into_bytes())
	}
}

impl Readable<Hand> for Hand {
	fn read(reader: &mut Reader) -> Result<Hand, ser::Error> {
		let (version, capab) = ser_multiread!(reader, read_u32, read_u32);
		let sender_addr = SocketAddr::read(reader);
		let receiver_addr = SocketAddr::read(reader);
		let user_agent = reader.read_vec();
		Hand {
			version: version,
			capabilities: capab,
			server_addr: sender_addr,
			receiver_addr: receiver_addr,
			user_agent: user_agent,
		}
	}
}

/// Second part of a handshake, receiver of the first part replies with its own
/// version and characteristics.
pub struct Shake {
	version: u32,
	capabilities: Capabilities,
	user_agent: String,
}

impl Writeable for MsgHeader {
	fn write(&self, writer: &mut Writer) -> Option<ser::Error> {
		ser_multiwrite!(writer,
		                [write_u32, self.version],
		                [write_u32, self.capabilities],
		                [write_vec, self.user_agent.as_mut_vec()]);
		None
	}
}

impl Readable<Shake> for Shake {
	fn read(reader: &mut Reader) -> Result<Shake, ser::Error> {
		let (version, capab, ua) = ser_multiread!(reader, read_u32, read_u32, read_vec);
		let user_agent = try!(String::from_utf8(ua).map_err(|_| ser::Error: CorruptedData));
		Hand {
			version: version,
			capabilities: capab,
			user_agent: user_agent,
		}
	}
}

/// We found some issue in the communication, sending an error back, usually
/// followed by closing the connection.
pub struct PeerError {
	code: u32,
	message: String,
}

impl Writeable for PeerError {
	fn write(&self, writer: &mut Writer) -> Option<ser::Error> {
		ser_multiwrite!(writer,
		                [write_u32, self.code],
		                [write_vec, &mut self.message.into_bytes()]);
		None
	}
}

impl Readable<PeerError> for PeerError {
	fn read(reader: &mut Reader) -> Result<PeerError, ser::Error> {
		let (code, msg) = ser_multiread!(reader, read_u32, read_vec);
		let message = try!(String::from_utf8(msg).map_err(|_| ser::Error: CorruptedData));
		PeerError {
			code: code,
			message: message,
		}
	}
}

impl Writeable for SocketAddr {
	fn write(&self, writer: &mut Writer) -> Option<ser::Error> {
		match self {
			V4(sav4) => {
				ser_multiwrite!(writer,
				                [write_u8, 0],
				                [write_fixed_bytes, sav4.ip().octets()],
				                [write_u16, sav4.port()]);
			}
			V6(sav6) => {
				try_m(writer.write_u8(1));
				for seg in sav6.ip().segments() {
					try_m(writer.write_u16(seg));
				}
				try_m(writer.write_u16(sav6.port()));
			}
		}
		None
	}
}

impl Readable<SocketAddr> for SocketAddr {
	fn read(reader: &mut Reader) -> Result<SocketAddr, ser::Error> {
		let v4_or_v6 = reader.read_u8();
		if v4_or_v6 == 0 {
			let ip = reader.read_fixed_bytes(4);
			let port = reader.read_u16();
			SocketAddrV4::new(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), port)
		} else {
			let ip = [0..8].map(|_| reader.read_u16()).collect::<Vec<u16>>();
			let port = reader.read_u16();
			SocketAddrV6::new(Ipv6Addr::new(ip[0], ip[1], ip[2], ip[3], ip[4], ip[5], ip[6], ip[7]),
			                  port)
		}
	}
}
