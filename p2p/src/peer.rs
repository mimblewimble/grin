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

use std::net::SocketAddr;
use std::io::{self, Read, Write};

use mioco::tcp::{TcpStream, Shutdown};

use handshake::Handshake;
use core::ser::Error;
use msg::*;
use types::*;

/// The local representation of a remotely connected peer. Handles most
/// low-level network communication and tracks peer information.
pub struct PeerConn {
	conn: TcpStream,
	pub capabilities: Capabilities,
	pub user_agent: String,
}

/// Make the Peer a Reader for convenient access to the underlying connection.
/// Allows the peer to track how much is received.
impl Read for PeerConn {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		self.conn.read(buf)
	}
}

/// Make the Peer a Writer for convenient access to the underlying connection.
/// Allows the peer to track how much is sent.
impl Write for PeerConn {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		self.conn.write(buf)
	}
  fn flush(&mut self) -> io::Result<()> {
    self.conn.flush()
  }
}

impl Close for PeerConn {
	fn close(&self) {
		self.conn.shutdown(Shutdown::Both);
	}
}

impl PeerConn {
	/// Create a new local peer instance connected to a remote peer with the
	/// provided TcpStream.
	pub fn new(conn: TcpStream) -> PeerConn {
		// don't wait on read for more than 2 seconds by default
		conn.set_keepalive(Some(2));

		PeerConn {
			conn: conn,
			capabilities: UNKNOWN,
			user_agent: "".to_string(),
		}
	}

	pub fn connect(&mut self, hs: &Handshake, na: &NetAdapter) -> Option<Error> {
		let mut proto = try_to_o!(hs.connect(self));
		proto.handle(na)
	}

	pub fn handshake(&mut self, hs: &Handshake, na: &NetAdapter) -> Option<Error> {
		let mut proto = try_to_o!(hs.handshake(self));
		proto.handle(na)
	}
}

impl PeerInfo for PeerConn {
	fn peer_addr(&self) -> SocketAddr {
		self.conn.peer_addr().unwrap()
	}
	fn local_addr(&self) -> SocketAddr {
    // TODO likely not exactly what we want (private vs public IP)
		self.conn.local_addr().unwrap()
	}
}
