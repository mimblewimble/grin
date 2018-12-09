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

/// Grin client commands processing
use std::net::SocketAddr;

use clap::ArgMatches;

use crate::api;
use crate::config::GlobalConfig;
use crate::p2p;
use crate::servers::ServerConfig;
use crate::util::file::get_first_line;
use term;

pub fn client_command(client_args: &ArgMatches<'_>, global_config: GlobalConfig) -> i32 {
	// just get defaults from the global config
	let server_config = global_config.members.unwrap().server;
	let api_secret = get_first_line(server_config.api_secret_path.clone());

	match client_args.subcommand() {
		("status", Some(_)) => {
			show_status(&server_config, api_secret);
		}
		("listconnectedpeers", Some(_)) => {
			list_connected_peers(&server_config, api_secret);
		}
		("ban", Some(peer_args)) => {
			let peer = peer_args.value_of("peer").unwrap();

			if let Ok(addr) = peer.parse() {
				ban_peer(&server_config, &addr, api_secret);
			} else {
				panic!("Invalid peer address format");
			}
		}
		("unban", Some(peer_args)) => {
			let peer = peer_args.value_of("peer").unwrap();

			if let Ok(addr) = peer.parse() {
				unban_peer(&server_config, &addr, api_secret);
			} else {
				panic!("Invalid peer address format");
			}
		}
		_ => panic!("Unknown client command, use 'grin help client' for details"),
	}
	0
}

pub fn show_status(config: &ServerConfig, api_secret: Option<String>) {
	println!();
	let title = format!("Grin Server Status");
	let mut t = term::stdout().unwrap();
	let mut e = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	writeln!(t, "--------------------------").unwrap();
	t.reset().unwrap();
	match get_status_from_node(config, api_secret) {
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
		)
		.unwrap(),
	};
	e.reset().unwrap();
	println!()
}

pub fn ban_peer(config: &ServerConfig, peer_addr: &SocketAddr, api_secret: Option<String>) {
	let params = "";
	let mut e = term::stdout().unwrap();
	let url = format!(
		"http://{}/v1/peers/{}/ban",
		config.api_http_addr,
		peer_addr.to_string()
	);
	match api::client::post_no_ret(url.as_str(), api_secret, &params).map_err(|e| Error::API(e)) {
		Ok(_) => writeln!(e, "Successfully banned peer {}", peer_addr.to_string()).unwrap(),
		Err(_) => writeln!(e, "Failed to ban peer {}", peer_addr).unwrap(),
	};
	e.reset().unwrap();
}

pub fn unban_peer(config: &ServerConfig, peer_addr: &SocketAddr, api_secret: Option<String>) {
	let params = "";
	let mut e = term::stdout().unwrap();
	let url = format!(
		"http://{}/v1/peers/{}/unban",
		config.api_http_addr,
		peer_addr.to_string()
	);
	let res: Result<(), api::Error>;
	res = api::client::post_no_ret(url.as_str(), api_secret, &params);

	match res.map_err(|e| Error::API(e)) {
		Ok(_) => writeln!(e, "Successfully unbanned peer {}", peer_addr).unwrap(),
		Err(_) => writeln!(e, "Failed to unban peer {}", peer_addr).unwrap(),
	};
	e.reset().unwrap();
}

pub fn list_connected_peers(config: &ServerConfig, api_secret: Option<String>) {
	let mut e = term::stdout().unwrap();
	let url = format!("http://{}/v1/peers/connected", config.api_http_addr);
	// let peers_info: Result<Vec<p2p::PeerInfoDisplay>, api::Error>;

	let peers_info = api::client::get::<Vec<p2p::types::PeerInfoDisplay>>(url.as_str(), api_secret);

	match peers_info.map_err(|e| Error::API(e)) {
		Ok(connected_peers) => {
			let mut index = 0;
			for connected_peer in connected_peers {
				writeln!(e, "Peer {}:", index).unwrap();
				writeln!(e, "Capabilities: {:?}", connected_peer.capabilities).unwrap();
				writeln!(e, "User agent: {}", connected_peer.user_agent).unwrap();
				writeln!(e, "Version: {}", connected_peer.version).unwrap();
				writeln!(e, "Peer address: {}", connected_peer.addr).unwrap();
				writeln!(e, "Height: {}", connected_peer.height).unwrap();
				writeln!(e, "Total difficulty: {}", connected_peer.total_difficulty).unwrap();
				writeln!(e, "Direction: {:?}", connected_peer.direction).unwrap();
				println!();
				index = index + 1;
			}
		}
		Err(_) => writeln!(e, "Failed to get connected peers").unwrap(),
	};

	e.reset().unwrap();
}

fn get_status_from_node(
	config: &ServerConfig,
	api_secret: Option<String>,
) -> Result<api::Status, Error> {
	let url = format!("http://{}/v1/status", config.api_http_addr);
	api::client::get::<api::Status>(url.as_str(), api_secret).map_err(|e| Error::API(e))
}

/// Error type wrapping underlying module errors.
#[derive(Debug)]
enum Error {
	/// Error originating from HTTP API calls.
	API(api::Error),
}
