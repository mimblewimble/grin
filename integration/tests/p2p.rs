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

//! P2P peer lifecycle: connect, ban, unban (node-side, no wallet).

#[macro_use]
extern crate log;

mod common;

use crate::common::{
	clean_all_output, init_chain, peer_addr, settle, stop_all_servers, LocalServerContainer,
	LocalServerContainerConfig,
};
use grin_p2p as p2p;
use grin_util as util;
use std::{thread, time};

/// Two nodes handshake; ban/unban updates peer store state.
#[test]
fn test_p2p_ban_unban() {
	util::init_test_logger();
	info!("starting test_p2p_ban_unban");
	init_chain();

	let server_one_dir = "p2p_server_one";
	clean_all_output(server_one_dir);
	let mut server_config_one = LocalServerContainerConfig::default();
	server_config_one.name = String::from(server_one_dir);
	server_config_one.p2p_server_port = 40002;
	server_config_one.api_server_port = 40003;
	server_config_one.start_miner = false;
	server_config_one.is_seeding = true;
	let server_one = LocalServerContainer::new(server_config_one.clone()).run_server();

	thread::sleep(time::Duration::from_millis(1000));

	let server_two_dir = "p2p_server_two";
	clean_all_output(server_two_dir);
	let mut server_config_two = LocalServerContainerConfig::default();
	server_config_two.name = String::from(server_two_dir);
	server_config_two.p2p_server_port = 40004;
	server_config_two.api_server_port = 40005;
	server_config_two.start_miner = false;
	server_config_two.is_seeding = false;
	let mut container_two = LocalServerContainer::new(server_config_two.clone());
	container_two.add_peer(format!(
		"{}:{}",
		server_config_one.base_addr, server_config_one.p2p_server_port
	));
	let server_two = container_two.run_server();

	// Handshake
	thread::sleep(time::Duration::from_millis(3000));

	assert_eq!(server_one.peer_count(), 1);

	let peer_two = peer_addr(&format!(
		"{}:{}",
		server_config_two.base_addr, server_config_two.p2p_server_port
	));

	// Peer is healthy in the store.
	let peer = server_one.p2p.peers.get_peer(peer_two).expect("peer known");
	assert_eq!(peer.flags, p2p::State::Healthy);

	// Ban
	server_one
		.p2p
		.peers
		.ban_peer(peer_two, p2p::ReasonForBan::ManualBan)
		.expect("ban");
	thread::sleep(time::Duration::from_millis(2000));

	let peer = server_one.p2p.peers.get_peer(peer_two).expect("peer after ban");
	assert_eq!(peer.flags, p2p::State::Banned);

	// Unban
	server_one.p2p.peers.unban_peer(peer_two).expect("unban");
	let peer = server_one
		.p2p
		.peers
		.get_peer(peer_two)
		.expect("peer after unban");
	assert_eq!(peer.flags, p2p::State::Healthy);

	// Ban drops the live connection; unban does not auto-reconnect.
	assert_eq!(server_one.peer_count(), 0);

	stop_all_servers(vec![server_one, server_two]);
	settle();
}
