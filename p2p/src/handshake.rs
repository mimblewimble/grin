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
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use futures::{self, Future};
use rand::Rng;
use rand::os::OsRng;
use tokio_core::net::TcpStream;

use core::core::target::Difficulty;
use core::core::hash::Hash;
use msg::*;
use types::*;
use protocol::ProtocolV1;
use util::LOGGER;

const NONCES_CAP: usize = 100;

/// Handles the handshake negotiation when two peers connect and decides on
/// protocol.
pub struct Handshake {
	/// Ring buffer of nonces sent to detect self connections without requiring
	/// a node id.
	nonces: Arc<RwLock<VecDeque<u64>>>,
	/// The genesis block header of the chain seen by this node.
	/// We only want to connect to other nodes seeing the same chain (forks are ok).
	genesis: Hash,
}

unsafe impl Sync for Handshake {}
unsafe impl Send for Handshake {}

impl Handshake {
	/// Creates a new handshake handler
	pub fn new(genesis: Hash) -> Handshake {
		Handshake {
			nonces: Arc::new(RwLock::new(VecDeque::with_capacity(NONCES_CAP))),
			genesis: genesis,
		}
	}

	/// Handles connecting to a new remote peer, starting the version handshake.
	pub fn connect(
		&self,
		capab: Capabilities,
		total_difficulty: Difficulty,
		self_addr: SocketAddr,
		conn: TcpStream,
	) -> Box<Future<Item = (TcpStream, ProtocolV1, PeerInfo), Error = Error>> {

		// prepare the first part of the handshake
		let nonce = self.next_nonce();
		let peer_addr = match conn.peer_addr() {
			Ok(pa) => pa,
			Err(e) => return Box::new(futures::future::err(Error::Connection(e))),
		};

		let hand = Hand {
			version: PROTOCOL_VERSION,
			capabilities: capab,
			nonce: nonce,
			genesis: self.genesis,
			total_difficulty: total_difficulty,
			sender_addr: SockAddr(self_addr),
			receiver_addr: SockAddr(peer_addr),
			user_agent: USER_AGENT.to_string(),
		};

		let genesis = self.genesis;

		// write and read the handshake response
		Box::new(
			write_msg(conn, hand, Type::Hand)
				.and_then(|conn| read_msg::<Shake>(conn))
				.and_then(move |(conn, shake)| {
					if shake.version != PROTOCOL_VERSION {
						Err(Error::ProtocolMismatch {
							us: PROTOCOL_VERSION,
							peer: shake.version,
						})
					} else if shake.genesis != genesis {
						Err(Error::GenesisMismatch {
							us: genesis,
							peer: shake.genesis,
						})
					} else {
						let peer_info = PeerInfo {
							capabilities: shake.capabilities,
							user_agent: shake.user_agent,
							addr: peer_addr,
							version: shake.version,
							total_difficulty: shake.total_difficulty,
						};

						info!(LOGGER, "Connected to peer {:?}", peer_info);

						// when more than one protocol version is supported, choosing should go here
						Ok((conn, ProtocolV1::new(), peer_info))
					}
				}),
		)
	}

	/// Handles receiving a connection from a new remote peer that started the
	/// version handshake.
	pub fn handshake(
		&self,
		capab: Capabilities,
		total_difficulty: Difficulty,
		conn: TcpStream,
	) -> Box<Future<Item = (TcpStream, ProtocolV1, PeerInfo), Error = Error>> {
		let nonces = self.nonces.clone();
		let genesis = self.genesis.clone();
		Box::new(
			read_msg::<Hand>(conn)
				.and_then(move |(conn, hand)| {
					if hand.version != PROTOCOL_VERSION {
						return Err(Error::ProtocolMismatch {
							us: PROTOCOL_VERSION,
							peer: hand.version,
						});
					} else if hand.genesis != genesis {
						return Err(Error::GenesisMismatch {
							us: genesis,
							peer: hand.genesis,
						});
					} else {
						// check the nonce to see if we are trying to connect to ourselves
						let nonces = nonces.read().unwrap();
						if nonces.contains(&hand.nonce) {
							return Err(Error::PeerWithSelf);
						}
					}

					// all good, keep peer info
					let peer_info = PeerInfo {
						capabilities: hand.capabilities,
						user_agent: hand.user_agent,
						addr: extract_ip(&hand.sender_addr.0, &conn),
						version: hand.version,
						total_difficulty: hand.total_difficulty,
					};
					// send our reply with our info
					let shake = Shake {
						version: PROTOCOL_VERSION,
						capabilities: capab,
						genesis: genesis,
						total_difficulty: total_difficulty,
						user_agent: USER_AGENT.to_string(),
					};
					Ok((conn, shake, peer_info))
				})
				.and_then(|(conn, shake, peer_info)| {
					debug!(LOGGER, "Success handshake with {}.", peer_info.addr);
					write_msg(conn, shake, Type::Shake)
					// when more than one protocol version is supported, choosing should go here
					.map(|conn| (conn, ProtocolV1::new(), peer_info))
				}),
		)
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

// Attempts to make a best guess at the correct remote IP by checking if the
// advertised address is the loopback and our TCP connection. Note that the
// port reported by the connection is always incorrect for receiving
// connections as it's dynamically allocated by the server.
fn extract_ip(advertised: &SocketAddr, conn: &TcpStream) -> SocketAddr {
  match advertised {
    &SocketAddr::V4(v4sock) => {
      let ip = v4sock.ip();
      if ip.is_loopback() || ip.is_unspecified() {
        if let Ok(addr) =  conn.peer_addr() {
          return SocketAddr::new(addr.ip(), advertised.port());
        }
      }
    }
    &SocketAddr::V6(v6sock) => {
      let ip = v6sock.ip();
      if ip.is_loopback() || ip.is_unspecified() {
        if let Ok(addr) =  conn.peer_addr() {
          return SocketAddr::new(addr.ip(), advertised.port());
        }
      }
    }
  }
  advertised.clone()
}
