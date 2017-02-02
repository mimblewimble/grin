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
	/// Initiates the handshake with another peer.
	pub fn connect(conn: TcpStream,
	               height: u64,
	               hs: &Handshake)
	               -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let connect_peer = hs.connect(height, conn).and_then(|(conn, proto, info)| {
			Ok((conn,
			    Peer {
				info: info,
				proto: Box::new(proto),
			}))
		});
		Box::new(connect_peer)
	}

	/// Accept a handshake initiated by another peer.
	pub fn accept(conn: TcpStream,
	              height: u64,
	              hs: &Handshake)
	              -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let hs_peer = hs.handshake(height, conn).and_then(|(conn, proto, info)| {
			Ok((conn,
			    Peer {
				info: info,
				proto: Box::new(proto),
			}))
		});
		Box::new(hs_peer)
	}

	/// Main peer loop listening for messages and forwarding to the rest of the
	/// system.
	pub fn run(&self,
	           conn: TcpStream,
	           na: Arc<NetAdapter>)
	           -> Box<Future<Item = (), Error = Error>> {

		self.proto.handle(conn, na)
	}

	/// Bytes sent and received by this peer to the remote peer.
	pub fn transmitted_bytes(&self) -> (u64, u64) {
		self.proto.transmitted_bytes()
	}

	pub fn send_ping(&self) -> Result<(), Error> {
		self.proto.send_ping()
	}

	/// Sends the provided block to the remote peer. The request may be dropped
	/// if the remote peer is known to already have the block.
	pub fn send_block(&self, b: &core::Block) -> Result<(), Error> {
		// TODO do not send if the peer sent us the block in the first place
		self.proto.send_block(b)
	}

	pub fn stop(&self) {
		self.proto.close();
	}
}
