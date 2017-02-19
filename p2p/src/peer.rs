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
use std::sync::Arc;

use futures::Future;
use tokio_core::net::TcpStream;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::ser::Error;
use handshake::Handshake;
use types::*;

pub struct Peer {
	pub info: PeerInfo,
	proto: Box<Protocol>,
}

unsafe impl Sync for Peer {}
unsafe impl Send for Peer {}

impl Peer {
	/// Initiates the handshake with another peer.
	pub fn connect(conn: TcpStream,
	               capab: Capabilities,
	               total_difficulty: Difficulty,
	               self_addr: SocketAddr,
	               hs: &Handshake)
	               -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let connect_peer = hs.connect(capab, total_difficulty, self_addr, conn)
			.and_then(|(conn, proto, info)| {
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
	              capab: Capabilities,
	              total_difficulty: Difficulty,
	              hs: &Handshake)
	              -> Box<Future<Item = (TcpStream, Peer), Error = Error>> {
		let hs_peer = hs.handshake(capab, total_difficulty, conn)
			.and_then(|(conn, proto, info)| {
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

		let addr = self.info.addr;
		Box::new(self.proto.handle(conn, na).and_then(move |_| {
			info!("Client {} disconnected.", addr);
			Ok(())
		}))
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

	pub fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error> {
		self.proto.send_header_request(locator)
	}

	pub fn send_block_request(&self, h: Hash) -> Result<(), Error> {
		debug!("Requesting block {} from peer {}.", h, self.info.addr);
		self.proto.send_block_request(h)
	}

	pub fn send_peer_request(&self, capab: Capabilities) -> Result<(), Error> {
		debug!("Asking {} for more peers.", self.info.addr);
		self.proto.send_peer_request(capab)
	}

	pub fn stop(&self) {
		self.proto.close();
	}
}
