// Copyright 2021 The Grin Developers
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

use crate::core::core::hash::Hash;
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::p2p::msg::built_info;
use crate::p2p::types::PeerAddr;
use grin_p2p::msg::PeerAddrs;
use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::{thread, time};

fn open_port() -> u16 {
	// use port 0 to allow the OS to assign an open port
	// TcpListener's Drop impl will unbind the port as soon as
	// listener goes out of scope
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	listener.local_addr().unwrap().port()
}

// Setup test with AutomatedTesting chain_type;
fn test_setup() {
	// Set "global" chain type here as we spawn peer threads for read/write.
	global::init_global_chain_type(global::ChainTypes::AutomatedTesting);
	util::init_test_logger();
}

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

fn p2p_server(
	dir: &str,
	peers_allow: Vec<PeerAddr>,
	peers_deny: Vec<PeerAddr>,
	port: Option<u16>,
) -> (SocketAddr, Arc<p2p::Server>) {
	let p2p_config = p2p::P2PConfig {
		host: "127.0.0.1".parse().unwrap(),
		port: port.unwrap_or_else(|| open_port()),
		peers_allow: if peers_allow.is_empty() {
			None
		} else {
			Some(PeerAddrs { peers: peers_allow })
		},
		peers_deny: if peers_deny.is_empty() {
			None
		} else {
			Some(PeerAddrs { peers: peers_deny })
		},
		..p2p::P2PConfig::default()
	};
	let net_adapter = Arc::new(p2p::DummyAdapter {});
	let server = Arc::new(
		p2p::Server::new(
			dir,
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

	let addr = SocketAddr::new(p2p_config.host, p2p_config.port);
	(addr, server)
}

#[test]
fn peer_handshake() {
	test_setup();
	let test_dir = "target/peer_handshake";
	clean_output_dir(test_dir);

	// Start peers and connect to check handshake, checking ping/pong exchange.
	{
		let (_, server) = p2p_server(test_dir, vec![], vec![], None);
		let (peer_addr, _) = p2p_server(test_dir, vec![], vec![], Some(5000));

		let peer = server.connect(PeerAddr(peer_addr)).unwrap();

		let git_hash =
			built_info::GIT_COMMIT_HASH_SHORT.map_or_else(|| "".to_owned(), |v| ".".to_owned() + v);
		assert!(peer
			.info
			.user_agent
			.ends_with(format!("{}{}", env!("CARGO_PKG_VERSION"), git_hash).as_str()));

		thread::sleep(time::Duration::from_secs(1));

		peer.send_ping(Difficulty::min_dma(), 0).unwrap();
		thread::sleep(time::Duration::from_secs(1));

		let server_peer = server
			.peers
			.get_connected_peer(PeerAddr(peer_addr))
			.unwrap();
		assert_eq!(server_peer.info.total_difficulty(), Difficulty::min_dma());
		assert!(server.peers.iter().connected().count() > 0);
	}

	// Start a server allowing connections from/to peer at "allow" list.
	{
		let allow_addr = PeerAddr("127.0.0.1:5002".parse().unwrap());
		let (addr, server) = p2p_server(test_dir, vec![allow_addr], vec![], Some(5001));

		let (addr2, server2) = p2p_server(test_dir, vec![], vec![], Some(5002));

		// Inbound connection test.
		let peer = server2.connect(PeerAddr(addr)).unwrap();
		peer.send_ping(Difficulty::min_dma(), 0).unwrap();
		thread::sleep(time::Duration::from_secs(1));

		assert!(server2.peers.iter().connected().count() > 0);

		server2
			.peers
			.disconnect_peer(PeerAddr(addr), "Inbound test finished")
			.unwrap();
		thread::sleep(time::Duration::from_secs(1));

		// Outbound connection test.
		let peer = server.connect(PeerAddr(addr2)).unwrap();
		peer.send_ping(Difficulty::min_dma(), 0).unwrap();
		thread::sleep(time::Duration::from_secs(1));

		let server_peer = server.peers.get_connected_peer(allow_addr).unwrap();
		assert_eq!(server_peer.info.total_difficulty(), Difficulty::min_dma());
		assert!(server.peers.iter().connected().count() > 0);

		server
			.peers
			.disconnect_peer(PeerAddr(addr2), "Outbound test finished")
			.unwrap();
		thread::sleep(time::Duration::from_secs(1));

		// Block connections from/to peer not from "allow" list.
		let (addr3, server3) = p2p_server(test_dir, vec![], vec![], Some(5003));

		assert!(server.connect(PeerAddr(addr3)).is_err());
		assert!(server3.connect(PeerAddr(addr)).is_err());
		assert_eq!(server.peers.iter().connected().count(), 0);
	}

	// Start a server to refuse peer from "deny" list.
	{
		let deny_addr = PeerAddr("127.0.0.1:5005".parse().unwrap());
		let (addr, server) = p2p_server(test_dir, vec![], vec![deny_addr], Some(5004));

		let (addr2, server2) = p2p_server(test_dir, vec![], vec![], Some(5005));

		// Inbound connection test.
		assert!(server2.connect(PeerAddr(addr)).is_err());
		assert_eq!(server.peers.iter().connected().count(), 0);

		// Outbound connection test.
		assert!(server.connect(PeerAddr(addr2)).is_err());
		assert_eq!(server.peers.iter().connected().count(), 0);
	}
}
