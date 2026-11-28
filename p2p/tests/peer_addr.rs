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

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use chrono::Utc;
use grin_core::global;
use grin_p2p as p2p;
use tempfile::tempdir;

use crate::p2p::store::PeerStore;
use crate::p2p::types::PeerAddr;

// Test the behavior of a hashmap of peers keyed by peer_addr.
#[test]
fn test_peer_addr_hashing() {
	let mut peers: HashMap<PeerAddr, String> = HashMap::new();

	let socket_addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(185, 147, 152, 14)), 8080);
	let peer_addr1 = PeerAddr(socket_addr1);
	peers.insert(peer_addr1, "peer1".into());

	assert!(peers.contains_key(&peer_addr1));
	assert_eq!(peers.len(), 1);

	let socket_addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(185, 147, 152, 14)), 8081);
	let peer_addr2 = PeerAddr(socket_addr2);

	// Expected behavior here is to ignore the port when hashing non-private peer_addr.
	// This means the two peer_addr instances above are seen as the same addr.
	assert!(peers.contains_key(&peer_addr1));
	assert!(peers.contains_key(&peer_addr2));

	peers.insert(peer_addr2, "peer2".into());

	// Inserting the second instance is a no-op as they are treated as the same addr.
	assert!(peers.contains_key(&peer_addr1));
	assert!(peers.contains_key(&peer_addr2));
	assert_eq!(peers.len(), 1);

	// Check they are treated as the same even though their underlying ports are different.
	assert_eq!(peer_addr1, peer_addr2);
	assert_eq!(peer_addr1.0, socket_addr1);
	assert_eq!(peer_addr2.0, socket_addr2);
	assert_eq!(peer_addr1.0.port(), 8080);
	assert_eq!(peer_addr2.0.port(), 8081);

	peers = HashMap::new();

	let socket_addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 8080);
	let peer_addr1 = PeerAddr(socket_addr1);
	peers.insert(peer_addr1, "peer1".into());

	assert!(peers.contains_key(&peer_addr1));
	assert_eq!(peers.len(), 1);

	let socket_addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 8081);
	let peer_addr2 = PeerAddr(socket_addr2);

	// Expected behavior here is to not ignore the port when hashing private peer_addr.
	// This means the two peer_addr instances above are seen as not the same addr.
	assert!(peers.contains_key(&peer_addr1));
	assert!(!peers.contains_key(&peer_addr2));

	peers.insert(peer_addr2, "peer2".into());

	// Inserting the second instance is a valid operation as they are treated as not the same addr.
	assert!(peers.contains_key(&peer_addr1));
	assert!(peers.contains_key(&peer_addr2));
	assert_eq!(peers.len(), 2);

	// Check they are treated as not the same even though their underlying ports are different.
	assert_ne!(peer_addr1, peer_addr2);
	assert_eq!(peer_addr1.as_key(), "192.168.1.2:8080");
	assert_eq!(peer_addr1.as_ip_key(), peer_addr2.as_ip_key());

	let mapped_addr = PeerAddr("[::ffff:192.168.1.2]:8082".parse().unwrap());
	assert_eq!(mapped_addr.as_key(), "192.168.1.2:8082");
	assert_eq!(peer_addr1.as_ip_key(), mapped_addr.as_ip_key());

	let mapped_same_port = PeerAddr("[::ffff:192.168.1.2]:8080".parse().unwrap());
	assert_eq!(peer_addr1, mapped_same_port);
	assert!(peers.contains_key(&mapped_same_port));

	let deny_addr = PeerAddr("192.168.1.2:0".parse().unwrap());
	assert!(deny_addr.matches_filter(&peer_addr2));
	assert!(deny_addr.matches_filter(&mapped_addr));

	let public_addr = PeerAddr("185.147.152.14:8080".parse().unwrap());
	let mapped_public_addr = PeerAddr("[::ffff:185.147.152.14]:8081".parse().unwrap());
	assert_eq!(public_addr, mapped_public_addr);
	assert_eq!(public_addr.as_key(), mapped_public_addr.as_key());
}

fn store_banned_peer(addr: PeerAddr) -> PeerStore {
	let dir = tempdir().unwrap();
	let store = PeerStore::new(dir.path().to_str().unwrap()).unwrap();
	let peer = p2p::PeerData {
		addr,
		capabilities: p2p::Capabilities::UNKNOWN,
		user_agent: "".to_string(),
		flags: p2p::State::Banned,
		last_banned: Utc::now().timestamp(),
		ban_reason: p2p::ReasonForBan::ManualBan,
		last_connected: 0,
		last_attempt: 0,
	};

	store.save_peer(&peer).unwrap();
	store
}

#[test]
fn test_peer_store_private_ip_key() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);

	let addr = PeerAddr("192.168.1.5:3414".parse().unwrap());
	let store = store_banned_peer(addr);

	assert!(store.exists_peer(addr).unwrap().0);
	assert_eq!(store.exists_peer(addr).unwrap().1, addr.as_key());
	assert_eq!(store.get_peer(addr).unwrap().0.flags, p2p::State::Banned);
	assert!(
		!store
			.exists_peer(PeerAddr("192.168.1.5:9999".parse().unwrap()))
			.unwrap()
			.0
	);

	store.unban_peer(addr).unwrap();
	assert!(!store.exists_peer(addr).unwrap().0);
}

#[test]
fn test_peer_store_public_ip_key() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);

	let addr = PeerAddr("185.168.1.5:3414".parse().unwrap());
	let store = store_banned_peer(addr);

	assert!(store.exists_peer(addr).unwrap().0);
	assert_eq!(store.exists_peer(addr).unwrap().1, addr.as_key());
	assert_eq!(store.get_peer(addr).unwrap().0.flags, p2p::State::Banned);
	assert!(
		store
			.exists_peer(PeerAddr("185.168.1.5:9999".parse().unwrap()))
			.unwrap()
			.0
	);

	store.unban_peer(addr).unwrap();
	assert!(store.exists_peer(addr).unwrap().0);
}
