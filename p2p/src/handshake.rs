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

use crate::util::RwLock;
use std::collections::VecDeque;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;

use chrono::prelude::*;
use rand::{thread_rng, Rng};

use crate::core::core::hash::Hash;
use crate::core::pow::Difficulty;
use crate::msg::{read_message, write_message, Hand, Shake, Type, PROTOCOL_VERSION, USER_AGENT};
use crate::peer::Peer;
use crate::types::{Capabilities, Direction, Error, P2PConfig, PeerAddr, PeerInfo, PeerLiveInfo};

/// Local generated nonce for peer connecting.
/// Used for self-connecting detection (on receiver side),
/// nonce(s) in recent 100 connecting requests are saved
const NONCES_CAP: usize = 100;
/// Socket addresses of self, extracted from stream when a self-connecting is detected.
/// Used in connecting request to avoid self-connecting request,
/// 10 should be enough since most of servers don't have more than 10 IP addresses.
const ADDRS_CAP: usize = 10;

/// Handles the handshake negotiation when two peers connect and decides on
/// protocol.
pub struct Handshake {
	/// Ring buffer of nonces sent to detect self connections without requiring
	/// a node id.
	nonces: Arc<RwLock<VecDeque<u64>>>,
	/// Ring buffer of self addr(s) collected from PeerWithSelf detection (by nonce).
	pub addrs: Arc<RwLock<VecDeque<PeerAddr>>>,
	/// The genesis block header of the chain seen by this node.
	/// We only want to connect to other nodes seeing the same chain (forks are
	/// ok).
	genesis: Hash,
	config: P2PConfig,
}

impl Handshake {
	/// Creates a new handshake handler
	pub fn new(genesis: Hash, config: P2PConfig) -> Handshake {
		Handshake {
			nonces: Arc::new(RwLock::new(VecDeque::with_capacity(NONCES_CAP))),
			addrs: Arc::new(RwLock::new(VecDeque::with_capacity(ADDRS_CAP))),
			genesis,
			config,
		}
	}

	pub fn initiate(
		&self,
		capab: Capabilities,
		total_difficulty: Difficulty,
		self_addr: PeerAddr,
		conn: &mut TcpStream,
	) -> Result<PeerInfo, Error> {
		// prepare the first part of the handshake
		let nonce = self.next_nonce();
		let peer_addr = match conn.peer_addr() {
			Ok(pa) => PeerAddr(pa),
			Err(e) => return Err(Error::Connection(e)),
		};

		let hand = Hand {
			version: PROTOCOL_VERSION,
			capabilities: capab,
			nonce: nonce,
			genesis: self.genesis,
			total_difficulty: total_difficulty,
			sender_addr: self_addr,
			receiver_addr: peer_addr,
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
			live_info: Arc::new(RwLock::new(PeerLiveInfo {
				total_difficulty: shake.total_difficulty,
				height: 0,
				last_seen: Utc::now(),
				stuck_detector: Utc::now(),
			})),
			direction: Direction::Outbound,
		};

		// If denied then we want to close the connection
		// (without providing our peer with any details why).
		if Peer::is_denied(&self.config, peer_info.addr) {
			return Err(Error::ConnectionClose);
		}

		debug!(
			"Connected! Cumulative {} offered from {:?} {:?} {:?}",
			shake.total_difficulty.to_num(),
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
			let nonces = self.nonces.read();
			let addr = resolve_peer_addr(hand.sender_addr, &conn);
			if nonces.contains(&hand.nonce) {
				// save ip addresses of ourselves
				let mut addrs = self.addrs.write();
				addrs.push_back(addr);
				if addrs.len() >= ADDRS_CAP {
					addrs.pop_front();
				}
				return Err(Error::PeerWithSelf);
			}
		}

		// all good, keep peer info
		let peer_info = PeerInfo {
			capabilities: hand.capabilities,
			user_agent: hand.user_agent,
			addr: resolve_peer_addr(hand.sender_addr, &conn),
			version: hand.version,
			live_info: Arc::new(RwLock::new(PeerLiveInfo {
				total_difficulty: hand.total_difficulty,
				height: 0,
				last_seen: Utc::now(),
				stuck_detector: Utc::now(),
			})),
			direction: Direction::Inbound,
		};

		// At this point we know the published ip and port of the peer
		// so check if we are configured to explicitly allow or deny it.
		// If denied then we want to close the connection
		// (without providing our peer with any details why).
		if Peer::is_denied(&self.config, peer_info.addr) {
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
		trace!("Success handshake with {}.", peer_info.addr);

		// when more than one protocol version is supported, choosing should go here
		Ok(peer_info)
	}

	/// Generate a new random nonce and store it in our ring buffer
	fn next_nonce(&self) -> u64 {
		let nonce = thread_rng().gen();

		let mut nonces = self.nonces.write();
		nonces.push_back(nonce);
		if nonces.len() >= NONCES_CAP {
			nonces.pop_front();
		}
		nonce
	}
}

/// Resolve the correct peer_addr based on the connection and the advertised port.
fn resolve_peer_addr(advertised: PeerAddr, conn: &TcpStream) -> PeerAddr {
	let port = advertised.0.port();
	if let Ok(addr) = conn.peer_addr() {
		PeerAddr(SocketAddr::new(addr.ip(), port))
	} else {
		advertised
	}
}
