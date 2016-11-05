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

extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate mioco;
extern crate env_logger;

mod common;

use std::io;
use std::time;

use core::core::*;
use p2p::Peer;
use common::*;

// Starts a server and connects a client peer to it to check handshake, followed by a ping/pong exchange to make sure the connection is live.
#[test]
fn peer_handshake() {
  env_logger::init().unwrap();

  with_server(|server| -> io::Result<()> {
    // connect a client peer to the server
    let peer = try!(connect_peer());

    // check server peer count
    let pc = server.peers_count();
    assert_eq!(pc, 1);

    // send a ping and check we got ponged (received data back)
    peer.send_ping();
    mioco::sleep(time::Duration::from_millis(50));
    let (sent, recv) = peer.transmitted_bytes();
    assert!(sent > 0);
    assert!(recv > 0);

    peer.stop();
    Ok(())
  });
}
