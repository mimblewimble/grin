// Copyright 2016-2018 The Grin Developers
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

use std::convert::From;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::ser;
use grin_store;

/// Maximum number of block headers a peer should ever send
pub const MAX_BLOCK_HEADERS: u32 = 512;

/// Maximum number of block bodies a peer should ever ask for and send
#[allow(dead_code)]
pub const MAX_BLOCK_BODIES: u32 = 16;

/// Maximum number of peer addresses a peer should ever send
pub const MAX_PEER_ADDRS: u32 = 256;

#[derive(Debug)]
pub enum Error {
	Serialization(ser::Error),
	Connection(io::Error),
	/// Header type does not match the expected message type
	BadMessage,
	Banned,
	ConnectionClose,
	Timeout,
	Store(grin_store::Error),
	PeerWithSelf,
	ProtocolMismatch {
		us: u32,
		peer: u32,
	},
	GenesisMismatch {
		us: Hash,
		peer: Hash,
	},
}

impl From<ser::Error> for Error {
	fn from(e: ser::Error) -> Error {
		Error::Serialization(e)
	}
}
impl From<grin_store::Error> for Error {
	fn from(e: grin_store::Error) -> Error {
		Error::Store(e)
	}
}
impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::Connection(e)
	}
}
impl<T> From<mpsc::SendError<T>> for Error {
	fn from(_e: mpsc::SendError<T>) -> Error {
		Error::ConnectionClose
	}
}
// impl From<TimerError> for Error {
// 	fn from(_: TimerError) -> Error {
// 		Error::Timeout
// 	}
// }

/// Configuration for the peer-to-peer server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PConfig {
	pub host: IpAddr,
	pub port: u16,

	pub peers_allow: Option<Vec<String>>,

	pub peers_deny: Option<Vec<String>>,
}

/// Default address for peer-to-peer connections.
impl Default for P2PConfig {
	fn default() -> P2PConfig {
		let ipaddr = "0.0.0.0".parse().unwrap();
		P2PConfig {
			host: ipaddr,
			port: 13414,
			peers_allow: None,
			peers_deny: None,
		}
	}
}

bitflags! {
  /// Options for what type of interaction a peer supports
  #[derive(Serialize, Deserialize)]
  pub struct Capabilities: u32 {
	/// We don't know (yet) what the peer can do.
	const UNKNOWN = 0b00000000;
	/// Full archival node, has the whole history without any pruning.
	const FULL_HIST = 0b00000001;
	/// Can provide block headers and the UTXO set for some recent-enough
	/// height.
	const UTXO_HIST = 0b00000010;
	/// Can provide a list of healthy peers
	const PEER_LIST = 0b00000100;

	const FULL_NODE = Capabilities::FULL_HIST.bits | Capabilities::UTXO_HIST.bits | Capabilities::PEER_LIST.bits;
  }
}

/// General information about a connected peer that's useful to other modules.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInfo {
	pub capabilities: Capabilities,
	pub user_agent: String,
	pub version: u32,
	pub addr: SocketAddr,
	pub total_difficulty: Difficulty,
}

/// Bridge between the networking layer and the rest of the system. Handles the
/// forwarding or querying of blocks and transactions from the network among
/// other things.
pub trait ChainAdapter: Sync + Send {
	/// Current total difficulty on our chain
	fn total_difficulty(&self) -> Difficulty;

	/// Current total height
	fn total_height(&self) -> u64;

	/// A valid transaction has been received from one of our peers
	fn transaction_received(&self, tx: core::Transaction);

	/// A block has been received from one of our peers. Returns true if the
	/// block could be handled properly and is not deemed defective by the
	/// chain. Returning false means the block will never be valid and
	/// may result in the peer being banned.
	fn block_received(&self, b: core::Block, addr: SocketAddr) -> bool;

	fn compact_block_received(&self, cb: core::CompactBlock, addr: SocketAddr) -> bool;

	fn header_received(&self, bh: core::BlockHeader, addr: SocketAddr) -> bool;

	/// A set of block header has been received, typically in response to a
	/// block
	/// header request.
	fn headers_received(&self, bh: Vec<core::BlockHeader>, addr: SocketAddr);

	/// Finds a list of block headers based on the provided locator. Tries to
	/// identify the common chain and gets the headers that follow it
	/// immediately.
	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader>;

	/// Gets a full block by its hash.
	fn get_block(&self, h: Hash) -> Option<core::Block>;
}

/// Additional methods required by the protocol that don't need to be
/// externally implemented.
pub trait NetAdapter: ChainAdapter {
	/// Find good peers we know with the provided capability and return their
	/// addresses.
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<SocketAddr>;

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, Vec<SocketAddr>);

	/// Heard total_difficulty from a connected peer (via ping/pong).
	fn peer_difficulty(&self, SocketAddr, Difficulty, u64);
}
