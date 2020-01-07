// Copyright 2019 The Grin Developers
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

use grin_core as core;
use grin_p2p as p2p;

use grin_util as util;
use grin_util::StopState;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::{thread, time};

use crate::core::core::hash::Hash;
use crate::core::pow::Difficulty;
use crate::p2p::types::PeerAddr;
use crate::p2p::Peer;
use grin_p2p::{DummyAdapter, P2PConfig, Server};

// Starts a server and connects a client peer to it to check handshake,
// followed by a ping/pong exchange to make sure the connection is live.
#[test]
fn peer_handshake() {
	util::init_test_logger();

	let (p2p_config, net_adapter, server) = setup_server();

	let peer1_addr = add_peer(&p2p_config, &net_adapter, "127.0.0.1:5000");
	let peer2_addr = add_peer(&p2p_config, &net_adapter, "127.0.0.1:5001");

	thread::sleep(time::Duration::from_secs(1));

	assert_eq!(server.peers.peer_count(), 2);

	// Send pings to all connected peers
	server.peers.check_all(Difficulty::min(), 0);
	thread::sleep(time::Duration::from_secs(1));

	let server_peer1 = server.peers.get_connected_peer(peer1_addr).unwrap();
	let server_peer2 = server.peers.get_connected_peer(peer2_addr).unwrap();

	assert!(server_peer1.info.ping_duration().is_some());
	assert!(server_peer2.info.ping_duration().is_some());

	// Make sure peer1 is always the slowest
	server_peer1.info.update_pinged();
	thread::sleep(time::Duration::from_millis(10));
	server_peer1.info.update(0, Difficulty::min(), true);

	assert_eq!(server_peer1.info.total_difficulty(), Difficulty::min());
	assert_eq!(
		server.peers.closest_most_work_peer().unwrap().info.addr,
		peer2_addr,
		"peer2 should be fastest"
	);
}

fn open_port() -> u16 {
	// use port 0 to allow the OS to assign an open port
	// TcpListener's Drop impl will unbind the port as soon as
	// listener goes out of scope
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.local_addr().unwrap().port()
}

fn setup_server() -> (P2PConfig, Arc<DummyAdapter>, Arc<Server>) {
	let p2p_config = p2p::P2PConfig {
		host: "127.0.0.1".parse().unwrap(),
		port: open_port(),
		peers_allow: None,
		peers_deny: None,
		..p2p::P2PConfig::default()
	};
	let net_adapter = Arc::new(p2p::DummyAdapter {});
	let server = Arc::new(
		p2p::Server::new(
			".grin",
			p2p::Capabilities::UNKNOWN,
			p2p_config.clone(),
			net_adapter.clone(),
			Hash::from_vec(&vec![]),
			Arc::new(StopState::new()),
		)
		.unwrap(),
	);

	let p2p_inner = server.clone();
	let _ = thread::spawn(move || p2p_inner.listen());

	thread::sleep(time::Duration::from_secs(1));
	(p2p_config, net_adapter, server)
}

fn add_peer(p2p_config: &P2PConfig, net_adapter: &Arc<DummyAdapter>, peer_addr: &str) -> PeerAddr {
	let addr = SocketAddr::new(p2p_config.host, p2p_config.port);
	let socket = TcpStream::connect_timeout(&addr, time::Duration::from_secs(10)).unwrap();
	let self_addr = PeerAddr(peer_addr.parse().unwrap());
	let peer = Peer::connect(
		socket,
		p2p::Capabilities::UNKNOWN,
		Difficulty::min(),
		self_addr,
		&p2p::handshake::Handshake::new(Hash::from_vec(&vec![]), p2p_config.clone()),
		net_adapter.clone(),
	)
	.unwrap();

	assert!(peer.info.user_agent.ends_with(env!("CARGO_PKG_VERSION")));
	self_addr
}
