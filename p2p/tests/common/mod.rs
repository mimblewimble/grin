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

use mioco;
use mioco::tcp::TcpStream;
use std::io;
use std::sync::Arc;
use std::time;
use p2p;
use p2p::Peer;

/// Server setup and teardown around the provided closure.
pub fn with_server<F>(closure: F) where F: Fn(Arc<p2p::Server>) -> io::Result<()>, F: Send + 'static {
  mioco::start(move || -> io::Result<()> {
    // start a server in its own coroutine
    let server = Arc::new(p2p::Server::new());
    let in_server = server.clone();
		mioco::spawn(move || -> io::Result<()> {
      try!(in_server.start().map_err(|_| io::Error::last_os_error()));
			Ok(())
		});

    // giving server a little time to start
    mioco::sleep(time::Duration::from_millis(50));

    try!(closure(server.clone()));

    server.stop();
    Ok(())
  }).unwrap().unwrap();
}

pub fn connect_peer() -> io::Result<Arc<Peer>> {
  let addr =  p2p::DEFAULT_LISTEN_ADDR.parse().unwrap();
  let tcp_client = TcpStream::connect(&addr).unwrap();
  let peer = try!(Peer::accept(tcp_client, &p2p::handshake::Handshake::new()).map_err(|_| io::Error::last_os_error()));
  mioco::sleep(time::Duration::from_millis(50));

  let peer = Arc::new(peer);
  let in_peer = peer.clone();
  mioco::spawn(move || -> io::Result<()> {
    in_peer.run(&p2p::DummyAdapter{});
    Ok(())
  });
  mioco::sleep(time::Duration::from_millis(50));
  Ok(peer.clone())
}
