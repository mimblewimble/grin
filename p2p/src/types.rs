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

use std::io::{Read, Write};
use std::net::SocketAddr;
use core::ser::Error;

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
/// ourselves and a remove) after handshake.
pub trait Protocol {
	/// Starts handling protocol communication, the peer(s) is expected to be
	/// known already, usually passed during construction.
	fn handle(&self, na: &NetAdapter) -> Option<Error>;
}

/// Bridge between the networking layer and the rest of the system. Handles the
/// forwarding or querying of blocks and transactions among other things.
pub trait NetAdapter {}
