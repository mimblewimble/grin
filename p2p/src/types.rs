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

/// Trait for pre-emptively and forcefully closing an underlying resource.
trait Close {
  fn close();
}

/// Main trait we expect a peer to implement to be usable by a Protocol.
trait Comm : Read + Write + Close;

/// A given communication protocol agreed upon between 2 peers (usually ourselves and a remove) after handshake.
trait Protocol {
  /// Instantiate providing a reader and writer allowing to communicate to the other peer.
  fn new(p: &mut Comm);

  /// Starts handling protocol communication, the peer(s) is expected to be known already, usually passed during construction.
  fn handle(&self, server: &Server);
}

