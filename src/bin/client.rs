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

extern crate term;

use std::net::SocketAddr;

use api;
use p2p;
use grin::ServerConfig;

pub fn show_status(config: &ServerConfig) {
	println!();
	let title = format!("Grin Server Status");
	let mut t = term::stdout().unwrap();
	let mut e = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	writeln!(t, "--------------------------").unwrap();
	t.reset().unwrap();
	match get_status_from_node(config) {
		Ok(status) => {
			writeln!(e, "Protocol version: {}", status.protocol_version).unwrap();
			writeln!(e, "User agent: {}", status.user_agent).unwrap();
			writeln!(e, "Connections: {}", status.connections).unwrap();
			writeln!(e, "Chain height: {}", status.tip.height).unwrap();
			writeln!(e, "Last block hash: {}", status.tip.last_block_pushed).unwrap();
			writeln!(e, "Previous block hash: {}", status.tip.prev_block_to_last).unwrap();
			writeln!(e, "Total difficulty: {}", status.tip.total_difficulty).unwrap();
		}
		Err(_) => writeln!(
			e,
			"WARNING: Client failed to get data. Is your `grin server` offline or broken?"
		).unwrap(),
	};
	e.reset().unwrap();
	println!()
}

pub fn ban_peer(config: &ServerConfig, peer_addr: &SocketAddr) {
	let params = "";
	let mut e = term::stdout().unwrap();
	let url = format!(
		"http://{}/v1/peers/{}/ban",
		config.api_http_addr,
		peer_addr.to_string()
	);
	match api::client::post(url.as_str(), &params).map_err(|e| Error::API(e)) {
		Ok(_) => writeln!(e, "Successfully banned peer {}", peer_addr.to_string()).unwrap(),
		Err(_) => writeln!(e, "Failed to ban peer {}", peer_addr).unwrap(),
	};
	e.reset().unwrap();
}

pub fn unban_peer(config: &ServerConfig, peer_addr: &SocketAddr) {
	let params = "";
	let mut e = term::stdout().unwrap();
	let url = format!(
		"http://{}/v1/peers/{}/unban",
		config.api_http_addr,
		peer_addr.to_string()
	);
	match api::client::post(url.as_str(), &params).map_err(|e| Error::API(e)) {
		Ok(_) => writeln!(e, "Successfully unbanned peer {}", peer_addr).unwrap(),
		Err(_) => writeln!(e, "Failed to unban peer {}", peer_addr).unwrap(),
	};
	e.reset().unwrap();
}

pub fn list_connected_peers(config: &ServerConfig) {
	let mut e = term::stdout().unwrap();
	let url = format!("http://{}/v1/peers/connected", config.api_http_addr);
	match api::client::get::<Vec<p2p::PeerInfo>>(url.as_str()).map_err(|e| Error::API(e)) {
		Ok(connected_peers) => {
			let mut index = 0;
			for connected_peer in connected_peers {
				writeln!(e, "Peer {}:", index).unwrap();
				writeln!(e, "Capabilities: {:?}", connected_peer.capabilities).unwrap();
				writeln!(e, "User agent: {}", connected_peer.user_agent).unwrap();
				writeln!(e, "Version: {}", connected_peer.version).unwrap();
				writeln!(e, "Peer address: {}", connected_peer.addr).unwrap();
				writeln!(e, "Total difficulty: {}", connected_peer.total_difficulty).unwrap();
				println!();
				index = index + 1;
			}
		}
		Err(_) => writeln!(e, "Failed to get connected peers").unwrap(),
	};
	e.reset().unwrap();
}

fn get_status_from_node(config: &ServerConfig) -> Result<api::Status, Error> {
	let url = format!("http://{}/v1/status", config.api_http_addr);
	api::client::get::<api::Status>(url.as_str()).map_err(|e| Error::API(e))
}

/// Error type wrapping underlying module errors.
#[derive(Debug)]
enum Error {
	/// Error originating from HTTP API calls.
	API(api::Error),
}
