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

use mioco::tcp::TcpStream;

use core::ser::Error;
use handshake::Handshake;
use types::*;

pub struct Peer {
	info: PeerInfo,
	proto: Box<Protocol>,
}

unsafe impl Sync for Peer {}
unsafe impl Send for Peer {}

impl Peer {
	pub fn connect(conn: TcpStream, hs: &Handshake) -> Result<Peer, Error> {
		let (proto, info) = try!(hs.connect(conn));
		Ok(Peer {
			info: info,
			proto: proto,
		})
	}

	pub fn accept(conn: TcpStream, hs: &Handshake) -> Result<Peer, Error> {
		let (proto, info) = try!(hs.handshake(conn));
		Ok(Peer {
			info: info,
			proto: proto,
		})
	}

	pub fn run(&self, na: &NetAdapter) -> Option<Error> {
		self.proto.handle(na)
	}

	pub fn send_ping(&self) -> Option<Error> {
		self.proto.send_ping()
	}

	pub fn transmitted_bytes(&self) -> (u64, u64) {
		self.proto.transmitted_bytes()
	}

	pub fn stop(&self) {
		self.proto.as_ref().close()
	}
}
