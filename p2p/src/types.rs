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

use std::convert::From;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::Future;
use tokio_core::net::TcpStream;
use tokio_timer::TimerError;

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
impl From<TimerError> for Error {
	fn from(_: TimerError) -> Error {
		Error::Timeout
	}
}

/// Configuration for the peer-to-peer server.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct P2PConfig {
	pub host: IpAddr,
	pub port: u16,
}

/// Default address for peer-to-peer connections.
impl Default for P2PConfig {
	fn default() -> P2PConfig {
		let ipaddr = "0.0.0.0".parse().unwrap();
		P2PConfig {
			host: ipaddr,
			port: 13414,
		}
	}
}

bitflags! {
  /// Options for what type of interaction a peer supports
  #[derive(Serialize, Deserialize)]
  pub flags Capabilities: u32 {
	/// We don't know (yet) what the peer can do.
	const UNKNOWN = 0b00000000,
	/// Full archival node, has the whole history without any pruning.
	const FULL_HIST = 0b00000001,
	/// Can provide block headers and the UTXO set for some recent-enough
	/// height.
	const UTXO_HIST = 0b00000010,
	/// Can provide a list of healthy peers
	const PEER_LIST = 0b00000100,

	const FULL_NODE = FULL_HIST.bits | UTXO_HIST.bits | PEER_LIST.bits,
  }
}

/// General information about a connected peer that's useful to other modules.
#[derive(Clone, Debug, Serialize)]
pub struct PeerInfo {
	pub capabilities: Capabilities,
	pub user_agent: String,
	pub version: u32,
	pub addr: SocketAddr,
	pub total_difficulty: Difficulty,
}

/// A given communication protocol agreed upon between 2 peers (usually
/// ourselves and a remote) after handshake. This trait is necessary to allow
/// protocol negotiation as it gets upgraded to multiple versions.
pub trait Protocol {
	/// Starts handling protocol communication, the connection) is expected to
	/// be  known already, usually passed during construction. Will typically
	/// block so needs to be called withing a coroutine. Should also be called
	/// only once.
	fn handle(&self, conn: TcpStream, na: Arc<NetAdapter>, addr: SocketAddr)
		-> Box<Future<Item = (), Error = Error>>;

	/// Sends a ping message to the remote peer.
	fn send_ping(&self, total_difficulty: Difficulty) -> Result<(), Error>;

	/// Relays a block to the remote peer.
	fn send_block(&self, b: &core::Block) -> Result<(), Error>;

	/// Relays a transaction to the remote peer.
	fn send_transaction(&self, tx: &core::Transaction) -> Result<(), Error>;

	/// Sends a request for block headers based on the provided block locator.
	fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error>;

	/// Sends a request for a block from its hash.
	fn send_block_request(&self, h: Hash) -> Result<(), Error>;

	/// Sends a request for some peer addresses.
	fn send_peer_request(&self, capab: Capabilities) -> Result<(), Error>;

	/// How many bytes have been sent/received to/from the remote peer.
	fn transmitted_bytes(&self) -> (u64, u64);

	/// Close the connection to the remote peer.
	fn close(&self);
}

/// Bridge between the networking layer and the rest of the system. Handles the
/// forwarding or querying of blocks and transactions from the network among
/// other things.
pub trait NetAdapter: Sync + Send {
	/// Current height of our chain.
	fn total_difficulty(&self) -> Difficulty;

	/// A valid transaction has been received from one of our peers
	fn transaction_received(&self, tx: core::Transaction);

	/// A block has been received from one of our peers
	fn block_received(&self, b: core::Block, addr: SocketAddr);

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

	/// Find good peers we know with the provided capability and return their
	/// addresses.
	fn find_peer_addrs(&self, capab: Capabilities) -> Vec<SocketAddr>;

	/// A list of peers has been received from one of our peers.
	fn peer_addrs_received(&self, Vec<SocketAddr>);

	/// Network successfully connected to a peer.
	fn peer_connected(&self, &PeerInfo);

	/// Heard total_difficulty from a connected peer (via ping/pong).
	fn peer_difficulty(&self, SocketAddr, Difficulty);
}
