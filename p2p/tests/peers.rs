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

mod fakes;

use crate::fakes::peer::FakePeerFactory;
use grin_core::pow::Difficulty;
use grin_p2p as p2p;
use grin_p2p::{PeerStore, Peers};
use grin_util as util;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[test]
fn test_most_work_peer() {
	util::init_test_logger();
	let peer_factory = FakePeerFactory::new();
	let peers = setup_peers();

	let fast_peer = peer_factory.build();
	let slow_peer = peer_factory.build();
	peers.add_connected(Arc::new(fast_peer.clone())).unwrap();
	peers.add_connected(Arc::new(slow_peer.clone())).unwrap();

	fast_peer.info.update_pinged();
	sleep(Duration::from_millis(1));
	fast_peer.info.update(0, Difficulty::min(), true);

	slow_peer.info.update_pinged();
	sleep(Duration::from_millis(10));
	slow_peer.info.update(0, Difficulty::min(), true);

	assert_eq!(peers.most_work_peers().len(), 2);
	assert_eq!(
		peers.most_work_peer().unwrap().info().addr,
		fast_peer.info.addr
	);
}

#[test]
fn test_connected_peers() {
	util::init_test_logger();
	let peer_factory = FakePeerFactory::new();
	let peers = setup_peers();

	peers.add_connected(Arc::new(peer_factory.build())).unwrap();
	peers.add_connected(Arc::new(peer_factory.build())).unwrap();

	assert_eq!(peers.connected_peers().len(), 2);
}

#[test]
fn test_is_known() {
	util::init_test_logger();
	let peer_factory = FakePeerFactory::new();
	let peers = setup_peers();

	let peer = peer_factory.build();
	peers.add_connected(Arc::new(peer.clone())).unwrap();

	assert!(peers.is_known(peer.info.addr).unwrap());
}

#[test]
fn test_is_banned() {
	util::init_test_logger();
	let peer_factory = FakePeerFactory::new();
	let peers = setup_peers();

	let peer = peer_factory.build();
	peers.add_connected(Arc::new(peer.clone())).unwrap();

	assert!(!peers.is_banned(peer.info.addr));
}

fn setup_peers() -> Peers {
	let p2p_config = p2p::P2PConfig {
		host: "127.0.0.1".parse().unwrap(),
		port: 0,
		peers_allow: None,
		peers_deny: None,
		..p2p::P2PConfig::default()
	};

	Peers::new(
		PeerStore::new(".grin").unwrap(),
		Arc::new(p2p::DummyAdapter {}),
		p2p_config.clone(),
	)
}
