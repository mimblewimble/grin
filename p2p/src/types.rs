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

use std::net::{SocketAddr, IpAddr};
use std::sync::Arc;

use futures::Future;
use tokio_core::net::TcpStream;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::ser::Error;

/// Maximum number of hashes in a block header locator request
pub const MAX_LOCATORS: u32 = 10;

/// Maximum number of block headers a peer should ever send
pub const MAX_BLOCK_HEADERS: u32 = 512;

/// Maximum number of block bodies a peer should ever ask for and send
pub const MAX_BLOCK_BODIES: u32 = 16;

/// Configuration for the peer-to-peer server.
#[derive(Debug, Clone, Copy)]
pub struct P2PConfig {
	pub host: IpAddr,
	pub port: u16,
}

/// Default address for peer-to-peer connections.
impl Default for P2PConfig {
	fn default() -> P2PConfig {
		let ipaddr = "127.0.0.1".parse().unwrap();
		P2PConfig {
			host: ipaddr,
			port: 13414,
		}
	}
}

bitflags! {
  /// Options for block validation
  pub flags Capabilities: u32 {
    /// We don't know (yet) what the peer can do.
    const UNKNOWN = 0b00000000,
    /// Full archival node, has the whole history without any pruning.
    const FULL_HIST = 0b00000001,
    /// Can provide block headers and the UTXO set for some recent-enough
    /// height.
    const UTXO_HIST = 0b00000010,
  }
}

/// General information about a connected peer that's useful to other modules.
#[derive(Debug)]
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
	fn handle(&self,
	          conn: TcpStream,
	          na: Arc<NetAdapter>)
	          -> Box<Future<Item = (), Error = Error>>;

	/// Sends a ping message to the remote peer.
	fn send_ping(&self) -> Result<(), Error>;

	/// Relays a block to the remote peer.
	fn send_block(&self, b: &core::Block) -> Result<(), Error>;

	/// Relays a transaction to the remote peer.
	fn send_transaction(&self, tx: &core::Transaction) -> Result<(), Error>;

	/// Sends a request for block headers based on the provided block locator.
	fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error>;

	/// Sends a request for a block from its hash.
	fn send_block_request(&self, h: Hash) -> Result<(), Error>;

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
	fn block_received(&self, b: core::Block);

	/// A set of block header has been received, typically in response to a
	/// block
	/// header request.
	fn headers_received(&self, bh: Vec<core::BlockHeader>);

	/// Finds a list of block headers based on the provided locator. Tries to
	/// identify the common chain and gets the headers that follow it
	/// immediately.
	fn locate_headers(&self, locator: Vec<Hash>) -> Vec<core::BlockHeader>;

	/// Gets a full block by its hash.
	fn get_block(&self, h: Hash) -> Option<core::Block>;
}
