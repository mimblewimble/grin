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

use grin_p2p as p2p;

use crate::p2p::types::PeerAddr;

// Test the behavior of a hashmap of peers keyed by peer_addr.
#[test]
fn test_peer_addr_hashing() {
	let mut peers: HashMap<PeerAddr, String> = HashMap::new();

	let socket_addr1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)), 8080);
	let peer_addr1 = PeerAddr(socket_addr1);
	peers.insert(peer_addr1, "peer1".into());

	assert!(peers.contains_key(&peer_addr1));
	assert_eq!(peers.len(), 1);

	let socket_addr2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1)), 8081);
	let peer_addr2 = PeerAddr(socket_addr2);

	// Expected behavior here is to ignore the port when hashing peer_addr.
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
}
