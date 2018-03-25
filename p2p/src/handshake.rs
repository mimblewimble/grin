// Copyright 2018 The Grin Developers
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
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, RwLock};

use rand::Rng;
use rand::os::OsRng;

use core::core::target::Difficulty;
use core::core::hash::Hash;
use msg::*;
use peer::Peer;
use types::*;
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
	config: P2PConfig,
}

unsafe impl Sync for Handshake {}
unsafe impl Send for Handshake {}

impl Handshake {
	/// Creates a new handshake handler
	pub fn new(genesis: Hash, config: P2PConfig) -> Handshake {
		Handshake {
			nonces: Arc::new(RwLock::new(VecDeque::with_capacity(NONCES_CAP))),
			genesis,
			config,
		}
	}

	pub fn initiate(
		&self,
		capab: Capabilities,
		total_difficulty: Difficulty,
		self_addr: SocketAddr,
		conn: &mut TcpStream,
	) -> Result<PeerInfo, Error> {
		// prepare the first part of the handshake
		let nonce = self.next_nonce();
		let peer_addr = match conn.peer_addr() {
			Ok(pa) => pa,
			Err(e) => return Err(Error::Connection(e)),
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

		// write and read the handshake response
		write_message(conn, hand, Type::Hand)?;
		let shake: Shake = read_message(conn, Type::Shake)?;
		if shake.version != PROTOCOL_VERSION {
			return Err(Error::ProtocolMismatch {
				us: PROTOCOL_VERSION,
				peer: shake.version,
			});
		} else if shake.genesis != self.genesis {
			return Err(Error::GenesisMismatch {
				us: self.genesis,
				peer: shake.genesis,
			});
		}
		let peer_info = PeerInfo {
			capabilities: shake.capabilities,
			user_agent: shake.user_agent,
			addr: peer_addr,
			version: shake.version,
			total_difficulty: shake.total_difficulty,
			direction: Direction::Outbound,
		};

		// If denied then we want to close the connection
		// (without providing our peer with any details why).
		if Peer::is_denied(&self.config, &peer_info.addr) {
			return Err(Error::ConnectionClose);
		}

		debug!(
			LOGGER,
			"Connected! Cumulative {} offered from {:?} {:?} {:?}",
			peer_info.total_difficulty.into_num(),
			peer_info.addr,
			peer_info.user_agent,
			peer_info.capabilities
		);
		// when more than one protocol version is supported, choosing should go here
		Ok(peer_info)
	}

	pub fn accept(
		&self,
		capab: Capabilities,
		total_difficulty: Difficulty,
		conn: &mut TcpStream,
	) -> Result<PeerInfo, Error> {
		let hand: Hand = read_message(conn, Type::Hand)?;

		// all the reasons we could refuse this connection for
		if hand.version != PROTOCOL_VERSION {
			return Err(Error::ProtocolMismatch {
				us: PROTOCOL_VERSION,
				peer: hand.version,
			});
		} else if hand.genesis != self.genesis {
			return Err(Error::GenesisMismatch {
				us: self.genesis,
				peer: hand.genesis,
			});
		} else {
			// check the nonce to see if we are trying to connect to ourselves
			let nonces = self.nonces.read().unwrap();
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
			direction: Direction::Inbound,
		};

		// At this point we know the published ip and port of the peer
		// so check if we are configured to explicitly allow or deny it.
		// If denied then we want to close the connection
		// (without providing our peer with any details why).
		if Peer::is_denied(&self.config, &peer_info.addr) {
			return Err(Error::ConnectionClose);
		}

		// send our reply with our info
		let shake = Shake {
			version: PROTOCOL_VERSION,
			capabilities: capab,
			genesis: self.genesis,
			total_difficulty: total_difficulty,
			user_agent: USER_AGENT.to_string(),
		};

		write_message(conn, shake, Type::Shake)?;
		trace!(LOGGER, "Success handshake with {}.", peer_info.addr);

		// when more than one protocol version is supported, choosing should go here
		Ok(peer_info)
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
				if let Ok(addr) = conn.peer_addr() {
					return SocketAddr::new(addr.ip(), advertised.port());
				}
			}
		}
		&SocketAddr::V6(v6sock) => {
			let ip = v6sock.ip();
			if ip.is_loopback() || ip.is_unspecified() {
				if let Ok(addr) = conn.peer_addr() {
					return SocketAddr::new(addr.ip(), advertised.port());
				}
			}
		}
	}
	advertised.clone()
}
