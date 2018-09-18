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

use std::net::SocketAddr;
use std::sync::{Arc, RwLock, Weak};
use std::thread;

use hyper::{Body, Request, StatusCode};
use rest::{Error, ErrorKind};

use chain;
use common::*;
use p2p;
use p2p::types::ReasonForBan;
use pool;
use rest::*;
use router::{Handler, ResponseFuture, Router, RouterError};
use types::*;
use util::LOGGER;

// All handlers use `Weak` references instead of `Arc` to avoid cycles that
// can never be destroyed. These 2 functions are simple helpers to reduce the
// boilerplate of dealing with `Weak`.
fn w<T>(weak: &Weak<T>) -> Arc<T> {
	weak.upgrade().unwrap()
}

// RESTful index of available api endpoints
// GET /v1/
struct IndexHandler {
	list: Vec<String>,
}

impl IndexHandler {}

impl Handler for IndexHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		json_response_pretty(&self.list)
	}
}

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
		let mut peers = vec![];
		for p in &w(&self.peers).connected_peers() {
			let p = p.read().unwrap();
			let peer_info = p.info.clone();
			peers.push(peer_info);
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
		let command = match req.uri().path().trim_right_matches("/").rsplit("/").next() {
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
		let mut path_elems = req.uri().path().trim_right_matches("/").rsplit("/");
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

/// Status handler. Post a summary of the server status
/// GET /v1/status
pub struct StatusHandler {
	pub chain: Weak<chain::Chain>,
	pub peers: Weak<p2p::Peers>,
}

impl StatusHandler {
	fn get_status(&self) -> Result<Status, Error> {
		let head = w(&self.chain)
			.head()
			.map_err(|e| ErrorKind::Internal(format!("can't get head: {}", e)))?;
		Ok(Status::from_tip_and_peers(
			head,
			w(&self.peers).peer_count(),
		))
	}
}

impl Handler for StatusHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_status())
	}
}

/// Chain validation handler.
/// GET /v1/chain/validate
pub struct ChainValidationHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainValidationHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		// TODO - read skip_rproofs from query params
		match w(&self.chain).validate(true) {
			Ok(_) => response(StatusCode::OK, ""),
			Err(e) => response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("validate failed: {}", e),
			),
		}
	}
}

/// Chain compaction handler. Trigger a compaction of the chain state to regain
/// storage space.
/// POST /v1/chain/compact
pub struct ChainCompactHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainCompactHandler {
	fn post(&self, _req: Request<Body>) -> ResponseFuture {
		match w(&self.chain).compact() {
			Ok(_) => response(StatusCode::OK, ""),
			Err(e) => response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("compact failed: {}", e),
			),
		}
	}
}

/// Get basic information about the transaction pool.
/// GET /v1/pool
struct PoolInfoHandler {
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
}

impl Handler for PoolInfoHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let pool_arc = w(&self.tx_pool);
		let pool = pool_arc.read().unwrap();

		json_response(&PoolInfo {
			pool_size: pool.total_size(),
		})
	}
}

/// Start all server HTTP handlers. Register all of them with Router
/// and runs the corresponding HTTP server.
///
/// Hyper currently has a bug that prevents clean shutdown. In order
/// to avoid having references kept forever by handlers, we only pass
/// weak references. Note that this likely means a crash if the handlers are
/// used after a server shutdown (which should normally never happen,
/// except during tests).
pub fn start_private_rest_apis(
	addr: String,
	chain: Weak<chain::Chain>,
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
	peers: Weak<p2p::Peers>,
) {
	let _ = thread::Builder::new()
		.name("private_apis".to_string())
		.spawn(move || {
			let mut apis = ApiServer::new();

			ROUTER.with(|router| {
				*router.borrow_mut() = Some(
					build_private_router(chain, tx_pool, peers)
						.expect("unable to build private API router"),
				);

				info!(LOGGER, "Starting private HTTP API server at {}.", addr);
				let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
				apis.start(socket_addr, &handle).unwrap_or_else(|e| {
					error!(LOGGER, "Failed to start private API HTTP server: {}.", e);
				});
			});
		});
}

pub fn build_private_router(
	chain: Weak<chain::Chain>,
	tx_pool: Weak<RwLock<pool::TransactionPool>>,
	peers: Weak<p2p::Peers>,
) -> Result<Router, RouterError> {
	let route_list = vec![
		"post chain/compact".to_string(),
		"post chain/validate".to_string(),
		"get status".to_string(),
		"get pool".to_string(),
		"post peers/a.b.c.d:p/ban".to_string(),
		"post peers/a.b.c.d:p/unban".to_string(),
		"get peers/all".to_string(),
		"get peers/connected".to_string(),
		"get peers/a.b.c.d".to_string(),
	];
	let index_handler = IndexHandler { list: route_list };

	let chain_compact_handler = ChainCompactHandler {
		chain: chain.clone(),
	};
	let chain_validation_handler = ChainValidationHandler {
		chain: chain.clone(),
	};
	let status_handler = StatusHandler {
		chain: chain.clone(),
		peers: peers.clone(),
	};
	let pool_info_handler = PoolInfoHandler {
		tx_pool: tx_pool.clone(),
	};
	let peers_all_handler = PeersAllHandler {
		peers: peers.clone(),
	};
	let peers_connected_handler = PeersConnectedHandler {
		peers: peers.clone(),
	};
	let peer_handler = PeerHandler {
		peers: peers.clone(),
	};

	let mut router = Router::new();
	router.add_route("/v1/", Box::new(index_handler))?;
	router.add_route("/v1/chain/compact", Box::new(chain_compact_handler))?;
	router.add_route("/v1/chain/validate", Box::new(chain_validation_handler))?;
	router.add_route("/v1/status", Box::new(status_handler))?;
	router.add_route("/v1/pool", Box::new(pool_info_handler))?;
	router.add_route("/v1/peers/all", Box::new(peers_all_handler))?;
	router.add_route("/v1/peers/connected", Box::new(peers_connected_handler))?;
	router.add_route("/v1/peers/**", Box::new(peer_handler))?;
	Ok(router)
}
