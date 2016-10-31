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

extern crate grin_p2p as p2p;
extern crate mioco;
extern crate env_logger;

use std::io;
use std::sync::Arc;
use std::time;

#[test]
fn peer_handshake() {
  env_logger::init().unwrap();

  mioco::start(|| -> io::Result<()> {
    // start a server in its own coroutine
    let server = Arc::new(p2p::Server::new());
    let in_server = server.clone();
		mioco::spawn(move || -> io::Result<()> {
      try!(in_server.start().map_err(|_| io::Error::last_os_error()));
			Ok(())
		});

    // giving server a little time to start
    mioco::sleep(time::Duration::from_millis(50));

    // connect a client peer to the server
    let addr =  p2p::DEFAULT_LISTEN_ADDR.parse().unwrap();
    let peer = try!(p2p::Server::connect_as_client(addr).map_err(|_| io::Error::last_os_error()));
    mioco::sleep(time::Duration::from_millis(50));
    assert_eq!(server.peers_count(), 1);

    // spawn our client peer to its own coroutine so it can poll for replies
    let peer = Arc::new(peer);
    let in_peer = peer.clone();
		mioco::spawn(move || -> io::Result<()> {
      in_peer.run(&p2p::DummyAdapter{});
      Ok(())
    });
    mioco::sleep(time::Duration::from_millis(50));

    // send a ping and check we got ponged
    peer.send_ping();
    mioco::sleep(time::Duration::from_millis(50));
    let (sent, recv) = peer.transmitted_bytes();
    assert!(sent > 0);
    assert!(recv > 0);

    server.stop();
    Ok(())
  }).unwrap().unwrap();
}
