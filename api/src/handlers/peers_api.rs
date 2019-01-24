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

use super::utils::w;
use crate::p2p;
use crate::p2p::types::{PeerInfoDisplay, ReasonForBan};
use crate::router::{Handler, ResponseFuture};
use crate::web::*;
use hyper::{Body, Request, StatusCode};
use std::sync::Weak;

pub struct PeersAllHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeersAllHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let peers = &w(&self.peers).all_peers();
		json_response_pretty(&peers)
	}
}

pub struct PeersConnectedHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeersConnectedHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let mut peers: Vec<PeerInfoDisplay> = vec![];
		for p in &w(&self.peers).connected_peers() {
			let peer_info = p.info.clone();
			peers.push(peer_info.into());
		}
		json_response(&peers)
	}
}

/// Peer operations
/// GET /v1/peers/10.12.12.13
/// POST /v1/peers/10.12.12.13/ban
/// POST /v1/peers/10.12.12.13/unban
pub struct PeerHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for PeerHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let command = match req.uri().path().trim_right_matches('/').rsplit('/').next() {
			Some(c) => c,
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
		};
		if let Ok(addr) = command.parse() {
			match w(&self.peers).get_peer(addr) {
				Ok(peer) => json_response(&peer),
				Err(_) => response(StatusCode::NOT_FOUND, "peer not found"),
			}
		} else {
			response(
				StatusCode::BAD_REQUEST,
				format!("peer address unrecognized: {}", req.uri().path()),
			)
		}
	}
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let mut path_elems = req.uri().path().trim_right_matches('/').rsplit('/');
		let command = match path_elems.next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(c) => c,
		};
		let addr = match path_elems.next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(a) => match a.parse() {
				Err(e) => {
					return response(
						StatusCode::BAD_REQUEST,
						format!("invalid peer address: {}", e),
					)
				}
				Ok(addr) => addr,
			},
		};

		match command {
			"ban" => w(&self.peers).ban_peer(&addr, ReasonForBan::ManualBan),
			"unban" => w(&self.peers).unban_peer(&addr),
			_ => return response(StatusCode::BAD_REQUEST, "invalid command"),
		};

		response(StatusCode::OK, "")
	}
}
