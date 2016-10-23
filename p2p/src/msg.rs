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

//! Message types that transit over the network and related serialization code.

use std::net::SocketAddr;

use core::ser::{Writeable, Readable, Writer, Reader};

mod ErrCodes {
	const UNSUPPORTED_VERSION: u32 = 100;
}

bitflags! {
  /// Options for block validation
  pub flags Capabilities: u32 {
    /// Runs with the easier version of the Proof of Work, mostly to make testing easier.
    const FULL_SYNC = 0b00000001,
  }
}

pub struct Hand {
	version: u32,
	capabilities: Capabilities,
	sender_addr: SocketAddr,
	receiver_addr: SocketAddr,
	user_agent: String,
}

pub struct Shake {
	version: u32,
	capabilities: Capabilities,
	user_agent: String,
}

pub struct PeerError {
	code: u32,
	message: String,
}
