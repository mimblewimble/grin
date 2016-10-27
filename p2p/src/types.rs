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

/// Trait for pre-emptively and forcefully closing an underlying resource.
pub trait Close {
	fn close(&self);
}

/// General information about a connected peer that's useful to other modules.
pub trait PeerInfo {
	/// Address of the remote peer
	fn peer_addr(&self) -> SocketAddr;
	/// Our address, communicated to other peers
	fn local_addr(&self) -> SocketAddr;
}

/// A given communication protocol agreed upon between 2 peers (usually
/// ourselves and a remove) after handshake.
pub trait Protocol {
	/// Starts handling protocol communication, the peer(s) is expected to be
	/// known already, usually passed during construction.
	fn handle(&mut self, na: &NetAdapter) -> Option<Error>;
}

/// Bridge between the networking layer and the rest of the system. Handles the
/// forwarding or querying of blocks and transactions among other things.
pub trait NetAdapter {}
