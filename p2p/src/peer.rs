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

use str::net::SocketAddr;
use std::str::FromStr;
use std::io::{Read, Write};

use time::Duration;

use mioco::tcp::{TcpListener, TcpStream, Shutdown};
use core::ser::{serialize, deserialize};

const PROTOCOL_VERSION: u32 = 1;
const USER_AGENT: &'static str = "MW/Grin 0.1";

/// The local representation of a remotely connected peer. Handles most
/// low-level network communication and tracks peer information.
struct Peer {
	conn: TcpStream,
	reader: BufReader,
	capabilities: Capabilities,
	user_agent: String,
}

/// Make the Peer a Reader for convenient access to the underlying connection.
/// Allows the peer to track how much is received.
impl Read for Peer {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
		self.reader.read(buf)
	}
}

/// Make the Peer a Writer for convenient access to the underlying connection.
/// Allows the peer to track how much is sent.
impl Write for Peer {
	fn write(&mut self, buf: &[u8]) -> Result<usize> {
		self.conf.write(buf)
	}
}

impl Close for Peer {
	fn close() {
		self.conn.shutdown(Shutdown::Both);
	}
}

impl Peer {
	/// Create a new local peer instance connected to a remote peer with the
	/// provided TcpStream.
	fn new(conn: TcpStream) -> Peer {
		// don't wait on read for more than 2 seconds by default
		conn.set_read_timeout(Some(Duration::seconds(2)));

		Peer {
			conn: conn,
			reader: BufReader::new(conn),
			capabilities: UNKNOWN,
			user_agent: "",
		}
	}

	/// Handles connecting to a new remote peer, starting the version handshake.
	fn connect(&mut self) -> Result<Protocol, Error> {
		serialize(self.peer,
		          &Hand {
			          version: PROTOCOL_VERSION,
			          capabilities: FULL_SYNC,
			          sender_addr: listen_addr(),
			          receiver_addr: self.peer.peer_addr(),
			          user_agent: USER_AGENT,
		          });
		let shake = deserialize(self.peer);
		if shake.version != 1 {
			self.close(ErrCodes::UNSUPPORTED_VERSION,
			           format!("Unsupported version: {}, ours: {})",
			                   shake.version,
			                   PROTOCOL_VERSION));
			return;
		}
		self.capabilities = shake.capabilities;
		self.user_agent = shake.user_agent;

		// when more than one protocol version is supported, choosing should go here
		ProtocolV1::new(&self);
	}

	/// Handles receiving a connection from a new remote peer that started the
	/// version handshake.
	fn handshake(&mut self) -> Result<Protocol, Error> {
		let hand = deserialize(self.peer);
		if hand.version != 1 {
			self.close(ErrCodes::UNSUPPORTED_VERSION,
			           format!("Unsupported version: {}, ours: {})",
			                   hand.version,
			                   PROTOCOL_VERSION));
			return;
		}

		self.peer.capabilities = hand.capabilities;
		self.peer.user_agent = hand.user_agent;

		serialize(self.peer,
		          &Shake {
			          version: PROTOCOL_VERSION,
			          capabilities: FULL_SYNC,
			          user_agent: USER_AGENT,
		          });
		self.accept_loop();

		// when more than one protocol version is supported, choosing should go here
		ProtocolV1::new(&self);
	}

	fn peer_addr(&self) -> SocketAddr {
		self.conn.peer_addr()
	}
}
