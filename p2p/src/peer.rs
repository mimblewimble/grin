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

use std::sync::Arc;

use futures::Future;
use tokio_core::net::TcpStream;

use core::core;
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
	pub fn connect(conn: TcpStream,
	               hs: &Handshake)
	               -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let connect_peer = hs.connect(conn).and_then(|(conn, proto, info)| {
			Ok((conn,
			    Peer {
				info: info,
				proto: Box::new(proto),
			}))
		});
		Box::new(connect_peer)
	}

	pub fn accept(conn: TcpStream,
	              hs: &Handshake)
	              -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let hs_peer = hs.handshake(conn).and_then(|(conn, proto, info)| {
			Ok((conn,
			    Peer {
				info: info,
				proto: Box::new(proto),
			}))
		});
		Box::new(hs_peer)
	}

	pub fn run(&self, conn: TcpStream, na: Arc<NetAdapter>) -> Box<Future<Item = (), Error = Error>> {
		self.proto.handle(conn, na)
	}

	pub fn transmitted_bytes(&self) -> (u64, u64) {
		self.proto.transmitted_bytes()
	}

	pub fn send_ping(&self) -> Result<(), Error> {
		self.proto.send_ping()
	}

	pub fn stop(&self) {
		self.proto.close();
	}
}
