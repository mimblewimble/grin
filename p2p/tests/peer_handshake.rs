// Copyright 2018 The Grin Developers
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
extern crate grin_util as util;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time;

use core::core::target::Difficulty;
use core::core::hash::Hash;
use p2p::Peer;

fn open_port() -> u16 {
	// use port 0 to allow the OS to assign an open port
	// TcpListener's Drop impl will unbind the port as soon as
	// listener goes out of scope
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.local_addr().unwrap().port()
}

// Starts a server and connects a client peer to it to check handshake,
// followed by a ping/pong exchange to make sure the connection is live.
#[test]
fn peer_handshake() {
	util::init_test_logger();

	let p2p_conf = p2p::P2PConfig {
		host: "0.0.0.0".parse().unwrap(),
		port: open_port(),
		peers_allow: None,
		peers_deny: None,
	};
	let net_adapter = Arc::new(p2p::DummyAdapter {});
	let server = Arc::new(
		p2p::Server::new(
			".grin".to_owned(),
			p2p::Capabilities::UNKNOWN,
			p2p_conf.clone(),
			net_adapter.clone(),
			Hash::from_vec(vec![]),
			Arc::new(AtomicBool::new(false)),
		).unwrap(),
	);

	let p2p_inner = server.clone();
	let _ = thread::spawn(move || p2p_inner.listen());

	thread::sleep(time::Duration::from_secs(1));

	let addr = SocketAddr::new(p2p_conf.host, p2p_conf.port);
	let mut socket = TcpStream::connect_timeout(&addr, time::Duration::from_secs(10)).unwrap();

	let my_addr = "127.0.0.1:5000".parse().unwrap();
	let mut peer = Peer::connect(
		&mut socket,
		p2p::Capabilities::UNKNOWN,
		Difficulty::one(),
		my_addr,
		&p2p::handshake::Handshake::new(Hash::from_vec(vec![]), p2p_conf.clone()),
		net_adapter,
	).unwrap();

	peer.start(socket);
	thread::sleep(time::Duration::from_secs(1));

	peer.send_ping(Difficulty::one(), 0).unwrap();
	thread::sleep(time::Duration::from_secs(1));

	let server_peer = server.peers.get_connected_peer(&my_addr).unwrap();
	let server_peer = server_peer.read().unwrap();
	assert_eq!(server_peer.info.total_difficulty, Difficulty::one());
	assert!(server.peers.peer_count() > 0);
}
