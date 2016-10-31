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
use std::sync::RwLock;

use rand::Rng;
use rand::os::OsRng;
use mioco::tcp::{TcpStream, Shutdown};

use core::ser::{serialize, deserialize, Error};
use msg::*;
use types::*;
use protocol::ProtocolV1;

const NONCES_CAP: usize = 100;

/// Handles the handshake negotiation when two peers connect and decides on
/// protocol.
pub struct Handshake {
	/// Ring buffer of nonces sent to detect self connections without requiring
	/// a node id.
	nonces: RwLock<VecDeque<u64>>,
}

unsafe impl Sync for Handshake {}
unsafe impl Send for Handshake {}

impl Handshake {
	/// Creates a new handshake handler
	pub fn new() -> Handshake {
		Handshake { nonces: RwLock::new(VecDeque::with_capacity(NONCES_CAP)) }
	}

	/// Handles connecting to a new remote peer, starting the version handshake.
	pub fn connect(&self, mut conn: TcpStream) -> Result<(Box<Protocol>, PeerInfo), Error> {
		// get a new nonce that can be used on handshake to detect self-connection
		let nonce = self.next_nonce();

		// send the first part of the handshake
		let sender_addr = conn.local_addr().unwrap();
		let receiver_addr = conn.peer_addr().unwrap();
		let opt_err = serialize(&mut conn,
		                        &Hand {
			                        version: PROTOCOL_VERSION,
			                        capabilities: FULL_SYNC,
			                        nonce: nonce,
			                        sender_addr: SockAddr(sender_addr),
			                        receiver_addr: SockAddr(receiver_addr),
			                        user_agent: USER_AGENT.to_string(),
		                        });
		match opt_err {
			Some(err) => return Err(err),
			None => {}
		}

		// deserialize the handshake response and do version negotiation
		let shake = try!(deserialize::<Shake>(&mut conn));
		if shake.version != 1 {
			self.close(&mut conn,
			           ErrCodes::UnsupportedVersion as u32,
			           format!("Unsupported version: {}, ours: {})",
			                   shake.version,
			                   PROTOCOL_VERSION));
			return Err(Error::UnexpectedData {
				expected: vec![PROTOCOL_VERSION as u8],
				received: vec![shake.version as u8],
			});
		}

		let peer_info = PeerInfo {
			capabilities: shake.capabilities,
			user_agent: shake.user_agent,
			addr: receiver_addr,
			version: shake.version,
		};

		info!("Connected to peer {:?}", peer_info);
		// when more than one protocol version is supported, choosing should go here
		Ok((Box::new(ProtocolV1::new(conn)), peer_info))
	}

	/// Handles receiving a connection from a new remote peer that started the
	/// version handshake.
	pub fn handshake(&self, mut conn: TcpStream) -> Result<(Box<Protocol>, PeerInfo), Error> {
		// deserialize first part of handshake sent to us and do version negotiation
		let hand = try!(deserialize::<Hand>(&mut conn));
		if hand.version != 1 {
			self.close(&mut conn,
			           ErrCodes::UnsupportedVersion as u32,
			           format!("Unsupported version: {}, ours: {})",
			                   hand.version,
			                   PROTOCOL_VERSION));
			return Err(Error::UnexpectedData {
				expected: vec![PROTOCOL_VERSION as u8],
				received: vec![hand.version as u8],
			});
		}
		{
			// check the nonce to see if we could be trying to connect to ourselves
			let nonces = self.nonces.read().unwrap();
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
		let opt_err = serialize(&mut conn,
		                        &Shake {
			                        version: PROTOCOL_VERSION,
			                        capabilities: FULL_SYNC,
			                        user_agent: USER_AGENT.to_string(),
		                        });
		match opt_err {
			Some(err) => return Err(err),
			None => {}
		}

		info!("Received connection from peer {:?}", peer_info);
		// when more than one protocol version is supported, choosing should go here
		Ok((Box::new(ProtocolV1::new(conn)), peer_info))
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

	fn close(&self, conn: &mut TcpStream, err_code: u32, explanation: String) {
		serialize(conn,
		          &PeerError {
			          code: err_code,
			          message: explanation,
		          });
		conn.shutdown(Shutdown::Both);
	}
}
