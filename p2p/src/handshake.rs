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

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use futures::Future;
use rand::Rng;
use rand::os::OsRng;
use tokio_core::net::TcpStream;

use core::ser::Error;
use msg::*;
use types::*;
use protocol::ProtocolV1;

const NONCES_CAP: usize = 100;

/// Handles the handshake negotiation when two peers connect and decides on
/// protocol.
pub struct Handshake {
	/// Ring buffer of nonces sent to detect self connections without requiring
	/// a node id.
	nonces: Arc<RwLock<VecDeque<u64>>>,
}

unsafe impl Sync for Handshake {}
unsafe impl Send for Handshake {}

impl Handshake {
	/// Creates a new handshake handler
	pub fn new() -> Handshake {
		Handshake { nonces: Arc::new(RwLock::new(VecDeque::with_capacity(NONCES_CAP))) }
	}

	/// Handles connecting to a new remote peer, starting the version handshake.
	pub fn connect(&self,
	               conn: TcpStream)
	               -> Box<Future<Item = (TcpStream, ProtocolV1, PeerInfo), Error = Error>> {
		// prepare the first part of the hanshake
		let nonce = self.next_nonce();
		let hand = Hand {
			version: PROTOCOL_VERSION,
			capabilities: FULL_SYNC,
			nonce: nonce,
			sender_addr: SockAddr(conn.local_addr().unwrap()),
			receiver_addr: SockAddr(conn.peer_addr().unwrap()),
			user_agent: USER_AGENT.to_string(),
		};

		// write and read the handshake response
		Box::new(write_msg(conn, hand, Type::Hand)
			.and_then(|conn| read_msg::<Shake>(conn))
			.and_then(|(conn, shake)| {
				if shake.version != 1 {
					Err(Error::UnexpectedData {
						expected: vec![PROTOCOL_VERSION as u8],
						received: vec![shake.version as u8],
					})
				} else {
					let peer_info = PeerInfo {
						capabilities: shake.capabilities,
						user_agent: shake.user_agent,
						addr: conn.peer_addr().unwrap(),
						version: shake.version,
					};

					info!("Connected to peer {:?}", peer_info);
					// when more than one protocol version is supported, choosing should go here
					Ok((conn, ProtocolV1::new(), peer_info))
				}
			}))
	}

	/// Handles receiving a connection from a new remote peer that started the
	/// version handshake.
	pub fn handshake(&self,
	                 conn: TcpStream)
	                 -> Box<Future<Item = (TcpStream, ProtocolV1, PeerInfo), Error = Error>> {
		let nonces = self.nonces.clone();
		Box::new(read_msg::<Hand>(conn)
			.and_then(move |(conn, hand)| {
				if hand.version != 1 {
					return Err(Error::UnexpectedData {
						expected: vec![PROTOCOL_VERSION as u8],
						received: vec![hand.version as u8],
					});
				}
				{
					// check the nonce to see if we could be trying to connect to ourselves
					let nonces = nonces.read().unwrap();
					if nonces.contains(&hand.nonce) {
						return Err(Error::UnexpectedData {
							expected: vec![],
							received: vec![],
						});
					}
				}
				// all good, keep peer info
				let peer_info = PeerInfo {
					capabilities: hand.capabilities,
					user_agent: hand.user_agent,
					addr: conn.peer_addr().unwrap(),
					version: hand.version,
				};
				// send our reply with our info
				let shake = Shake {
					version: PROTOCOL_VERSION,
					capabilities: FULL_SYNC,
					user_agent: USER_AGENT.to_string(),
				};
				Ok((conn, shake, peer_info))
			})
			.and_then(|(conn, shake, peer_info)| {
				write_msg(conn, shake, Type::Shake)
				  // when more than one protocol version is supported, choosing should go here
					.map(|conn| (conn, ProtocolV1::new(), peer_info))
			}))
	}

	/// Generate a new random nonce and store it in our ring buffer
	fn next_nonce(&self) -> u64 {
		let mut rng = OsRng::new().unwrap();
		let nonce = rng.next_u64();

		let mut nonces = self.nonces.write().unwrap();
		nonces.push_back(nonce);
		if nonces.len() >= NONCES_CAP {
			nonces.pop_front();
		}
		nonce
	}
}
