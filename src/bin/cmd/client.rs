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

/// Grin client commands processing
use std::net::SocketAddr;

use clap::ArgMatches;

use crate::api::client;
use crate::api::json_rpc::*;
use crate::api::types::Status;
use crate::config::GlobalConfig;
use crate::p2p::types::PeerInfoDisplay;
use crate::util::file::get_first_line;
use serde_json::json;

const ENDPOINT: &str = "/v2/owner";

#[derive(Clone)]
pub struct HTTPNodeClient {
	node_url: String,
	node_api_secret: Option<String>,
}
impl HTTPNodeClient {
	/// Create a new client that will communicate with the given grin node
	pub fn new(node_url: &str, node_api_secret: Option<String>) -> HTTPNodeClient {
		HTTPNodeClient {
			node_url: node_url.to_owned(),
			node_api_secret: node_api_secret,
		}
	}
	fn send_json_request<D: serde::de::DeserializeOwned>(
		&self,
		method: &str,
		params: &serde_json::Value,
	) -> Result<D, Error> {
		let timeout = match method {
			// 6 hours read timeout
			"validate_chain" => client::TimeOut::new(20, 21600, 20),
			_ => client::TimeOut::default(),
		};
		let url = format!("http://{}{}", self.node_url, ENDPOINT);
		let req = build_request(method, params);
		let res = client::post::<Request, Response>(
			url.as_str(),
			self.node_api_secret.clone(),
			&req,
			timeout,
		);

		match res {
			Err(e) => {
				let report = format!("Error calling {}: {}", method, e);
				error!("{}", report);
				Err(Error::RPCError(report))
			}
			Ok(inner) => match inner.clone().into_result() {
				Ok(r) => Ok(r),
				Err(e) => {
					error!("{:?}", inner);
					let report = format!("Unable to parse response for {}: {}", method, e);
					error!("{}", report);
					Err(Error::RPCError(report))
				}
			},
		}
	}

	pub fn show_status(&self) {
		println!();
		let title = "Grin Server Status".to_string();
		if term::stdout().is_none() {
			println!("Could not open terminal");
			return;
		}
		let mut t = term::stdout().unwrap();
		let mut e = term::stdout().unwrap();
		t.fg(term::color::MAGENTA).unwrap();
		writeln!(t, "{}", title).unwrap();
		writeln!(t, "--------------------------").unwrap();
		t.reset().unwrap();
		match self.send_json_request::<Status>("get_status", &serde_json::Value::Null) {
			Ok(status) => {
				writeln!(e, "Protocol version: {:?}", status.protocol_version).unwrap();
				writeln!(e, "User agent: {}", status.user_agent).unwrap();
				writeln!(e, "Connections: {}", status.connections).unwrap();
				writeln!(e, "Chain height: {}", status.tip.height).unwrap();
				writeln!(e, "Last block hash: {}", status.tip.last_block_pushed).unwrap();
				writeln!(e, "Previous block hash: {}", status.tip.prev_block_to_last).unwrap();
				writeln!(e, "Total difficulty: {}", status.tip.total_difficulty).unwrap();
				writeln!(e, "Sync status: {}", status.sync_status).unwrap();
				if let Some(sync_info) = status.sync_info {
					writeln!(e, "Sync info: {}", sync_info).unwrap();
				}
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

	pub fn list_connected_peers(&self) {
		let mut e = term::stdout().unwrap();
		match self.send_json_request::<Vec<PeerInfoDisplay>>(
			"get_connected_peers",
			&serde_json::Value::Null,
		) {
			Ok(connected_peers) => {
				for (index, connected_peer) in connected_peers.into_iter().enumerate() {
					writeln!(e, "Peer {}:", index).unwrap();
					writeln!(e, "Capabilities: {:?}", connected_peer.capabilities).unwrap();
					writeln!(e, "User agent: {}", connected_peer.user_agent).unwrap();
					writeln!(e, "Version: {:?}", connected_peer.version).unwrap();
					writeln!(e, "Peer address: {}", connected_peer.addr).unwrap();
					writeln!(e, "Height: {}", connected_peer.height).unwrap();
					writeln!(e, "Total difficulty: {}", connected_peer.total_difficulty).unwrap();
					writeln!(e, "Direction: {:?}", connected_peer.direction).unwrap();
					println!();
				}
			}
			Err(_) => writeln!(e, "Failed to get connected peers").unwrap(),
		};
		e.reset().unwrap();
	}

	pub fn reset_chain_head(&self, hash: String) {
		let mut e = term::stdout().unwrap();
		let params = json!([hash]);
		match self.send_json_request::<()>("reset_chain_head", &params) {
			Ok(_) => writeln!(e, "Successfully reset chain head {}", hash).unwrap(),
			Err(_) => writeln!(e, "Failed to reset chain head {}", hash).unwrap(),
		}
		e.reset().unwrap();
	}

	pub fn invalidate_header(&self, hash: String) {
		let mut e = term::stdout().unwrap();
		let params = json!([hash]);
		match self.send_json_request::<()>("invalidate_header", &params) {
			Ok(_) => writeln!(e, "Successfully invalidated header: {}", hash).unwrap(),
			Err(_) => writeln!(e, "Failed to invalidate header: {}", hash).unwrap(),
		}
		e.reset().unwrap();
	}

	pub fn verify_chain(&self, assume_valid_rangeproofs_kernels: bool) {
		let mut e = term::stdout().unwrap();
		let params = json!([assume_valid_rangeproofs_kernels]);
		writeln!(
			e,
			"Checking the state of the chain. This might take time..."
		)
		.unwrap();
		match self.send_json_request::<()>("validate_chain", &params) {
			Ok(_) => {
				if assume_valid_rangeproofs_kernels {
					writeln!(e, "Successfully validated the sum of kernel excesses! [fast_verification enabled]").unwrap()
				} else {
					writeln!(e, "Successfully validated the sum of kernel excesses, kernel signature and rangeproofs!").unwrap()
				}
			}
			Err(err) => writeln!(e, "Failed to validate chain: {:?}", err).unwrap(),
		}
		e.reset().unwrap();
	}

	pub fn ban_peer(&self, peer_addr: &SocketAddr) {
		let mut e = term::stdout().unwrap();
		let params = json!([peer_addr]);
		match self.send_json_request::<()>("ban_peer", &params) {
			Ok(_) => writeln!(e, "Successfully banned peer {}", peer_addr).unwrap(),
			Err(_) => writeln!(e, "Failed to ban peer {}", peer_addr).unwrap(),
		};
		e.reset().unwrap();
	}

	pub fn unban_peer(&self, peer_addr: &SocketAddr) {
		let mut e = term::stdout().unwrap();
		let params = json!([peer_addr]);
		match self.send_json_request::<()>("unban_peer", &params) {
			Ok(_) => writeln!(e, "Successfully unbanned peer {}", peer_addr).unwrap(),
			Err(_) => writeln!(e, "Failed to unban peer {}", peer_addr).unwrap(),
		};
		e.reset().unwrap();
	}
}

pub fn client_command(client_args: &ArgMatches<'_>, global_config: GlobalConfig) -> i32 {
	// just get defaults from the global config
	let server_config = global_config.members.unwrap().server;
	let api_secret = get_first_line(server_config.api_secret_path.clone());
	let node_client = HTTPNodeClient::new(&server_config.api_http_addr, api_secret);

	match client_args.subcommand() {
		("status", Some(_)) => {
			node_client.show_status();
		}
		("listconnectedpeers", Some(_)) => {
			node_client.list_connected_peers();
		}
		("resetchainhead", Some(args)) => {
			let hash = args.value_of("hash").unwrap();
			node_client.reset_chain_head(hash.to_string());
		}
		("invalidateheader", Some(args)) => {
			let hash = args.value_of("hash").unwrap();
			node_client.invalidate_header(hash.to_string());
		}
		("verify-chain", Some(args)) => {
			let assume_valid_rangeproofs_kernels = args.is_present("fast");
			node_client.verify_chain(assume_valid_rangeproofs_kernels);
		}
		("ban", Some(peer_args)) => {
			let peer = peer_args.value_of("peer").unwrap();

			if let Ok(addr) = peer.parse() {
				node_client.ban_peer(&addr);
			} else {
				panic!("Invalid peer address format");
			}
		}
		("unban", Some(peer_args)) => {
			let peer = peer_args.value_of("peer").unwrap();

			if let Ok(addr) = peer.parse() {
				node_client.unban_peer(&addr);
			} else {
				panic!("Invalid peer address format");
			}
		}
		_ => panic!("Unknown client command, use 'grin help client' for details"),
	}
	0
}
/// Error type wrapping underlying module errors.
#[derive(Debug)]
enum Error {
	/// RPC Error
	RPCError(String),
}
