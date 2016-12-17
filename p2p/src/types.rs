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
use core::ser::Error;

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
    /// Runs with the easier version of the Proof of Work, mostly to make testing easier.
    const FULL_SYNC = 0b00000001,
  }
}

/// General information about a connected peer that's useful to other modules.
#[derive(Debug)]
pub struct PeerInfo {
	pub capabilities: Capabilities,
	pub user_agent: String,
	pub version: u32,
	pub addr: SocketAddr,
}

/// A given communication protocol agreed upon between 2 peers (usually
/// ourselves and a remote) after handshake. This trait is necessary to allow
/// protocol negotiation as it gets upgraded to multiple versions.
pub trait Protocol {
	/// Starts handling protocol communication, the connection) is expected to
	/// be  known already, usually passed during construction. Will typically
	/// block so needs to be called withing a coroutine. Should also be called
	/// only once.
	fn handle(&self, conn: TcpStream, na: Arc<NetAdapter>) -> Box<Future<Item = (), Error = Error>>;

	/// Sends a ping message to the remote peer.
	fn send_ping(&self) -> Result<(), Error>;

	/// Relays a block to the remote peer.
	fn send_block(&self, b: &core::Block) -> Result<(), Error>;

	/// Relays a transaction to the remote peer.
	fn send_transaction(&self, tx: &core::Transaction) -> Result<(), Error>;

	/// How many bytes have been sent/received to/from the remote peer.
	fn transmitted_bytes(&self) -> (u64, u64);

	/// Close the connection to the remote peer.
	fn close(&self);
}

/// Bridge between the networking layer and the rest of the system. Handles the
/// forwarding or querying of blocks and transactions among other things.
pub trait NetAdapter {
	/// A valid transaction has been received from one of our peers
	fn transaction_received(&self, tx: core::Transaction);

	/// A block has been received from one of our peers
	fn block_received(&self, b: core::Block);
}
