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

extern crate futures;
extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate tokio_core;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time;

use futures::future::Future;
use tokio_core::net::TcpStream;
use tokio_core::reactor::{self, Core};

use core::core::target::Difficulty;
use p2p::Peer;

// Starts a server and connects a client peer to it to check handshake,
// followed by a ping/pong exchange to make sure the connection is live.
#[test]
fn peer_handshake() {
	let mut evtlp = Core::new().unwrap();
	let handle = evtlp.handle();
	let p2p_conf = p2p::P2PConfig::default();
	let net_adapter = Arc::new(p2p::DummyAdapter {});
	let server = p2p::Server::new(p2p::UNKNOWN, p2p_conf, net_adapter.clone());
	let run_server = server.start(handle.clone());
	let my_addr = "127.0.0.1:5000".parse().unwrap();

	let phandle = handle.clone();
	let rhandle = handle.clone();
	let timeout = reactor::Timeout::new(time::Duration::new(1, 0), &handle).unwrap();
	let timeout_send = reactor::Timeout::new(time::Duration::new(2, 0), &handle).unwrap();
	handle.spawn(
		timeout
			.from_err()
			.and_then(move |_| {
				let p2p_conf = p2p::P2PConfig::default();
				let addr = SocketAddr::new(p2p_conf.host, p2p_conf.port);
				let socket =
					TcpStream::connect(&addr, &phandle).map_err(|e| p2p::Error::Connection(e));
				socket
					.and_then(move |socket| {
						Peer::connect(
							socket,
							p2p::UNKNOWN,
							Difficulty::one(),
							my_addr,
							&p2p::handshake::Handshake::new(),
							net_adapter.clone(),
						)
					})
					.and_then(move |(socket, peer)| {
						rhandle.spawn(peer.run(socket).map_err(|e| {
							panic!("Client run failed: {:?}", e);
						}));
						peer.send_ping().unwrap();
						timeout_send.from_err().map(|_| peer)
					})
					.and_then(|peer| {
						let (sent, recv) = peer.transmitted_bytes();
						assert!(sent > 0);
						assert!(recv > 0);
						Ok(())
					})
					.and_then(|_| {
						assert!(server.peer_count() > 0);
						server.stop();
						Ok(())
					})
			})
			.map_err(|e| {
				panic!("Client connection failed: {:?}", e);
			}),
	);

	evtlp.run(run_server).unwrap();
}
