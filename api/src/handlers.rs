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

pub mod blocks_api;
pub mod chain_api;
pub mod peers_api;
pub mod pool_api;
pub mod server_api;
pub mod transactions_api;
pub mod utils;
pub mod version_api;

use crate::auth::{
	BasicAuthMiddleware, BasicAuthURIMiddleware, GRIN_BASIC_REALM, GRIN_FOREIGN_BASIC_REALM,
};
use crate::chain;
use crate::chain::{Chain, SyncState};
use crate::foreign::Foreign;
use crate::foreign_rpc::ForeignRpc;
use crate::owner::Owner;
use crate::owner_rpc::OwnerRpc;
use crate::p2p;
use crate::pool;
use crate::pool::{BlockChain, PoolAdapter};
use crate::rest::{ApiServer, Error, TLSConfig};
use crate::router::ResponseFuture;
use crate::router::Router;
use crate::util::to_base64;
use crate::util::RwLock;
use crate::util::StopState;
use crate::web::*;
use easy_jsonrpc_mw::{Handler, MaybeReply};
use futures::channel::oneshot;
use hyper::{Body, Request, Response, StatusCode};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::thread;

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn node_apis<B, P>(
	addr: &str,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<B, P>>>,
	peers: Arc<p2p::Peers>,
	sync_state: Arc<chain::SyncState>,
	api_secret: Option<String>,
	foreign_api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
	api_chan: &'static mut (oneshot::Sender<()>, oneshot::Receiver<()>),
	stop_state: Arc<StopState>,
) -> Result<(), Error>
where
	B: BlockChain + 'static,
	P: PoolAdapter + 'static,
{
	let mut router = Router::new();

	// Add basic auth to v2 owner API
	if let Some(api_secret) = api_secret {
		let api_basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret));
		let basic_auth_middleware = Arc::new(BasicAuthMiddleware::new(
			api_basic_auth,
			&GRIN_BASIC_REALM,
			Some("/v2/foreign".into()),
		));
		router.add_middleware(basic_auth_middleware);
	}

	let api_handler = OwnerAPIHandlerV2::new(
		Arc::downgrade(&chain),
		Arc::downgrade(&peers),
		Arc::downgrade(&sync_state),
	);
	router.add_route("/v2/owner", Arc::new(api_handler))?;

	// Add basic auth to v2 foreign API
	if let Some(api_secret) = foreign_api_secret {
		let api_basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret));
		let basic_auth_middleware = Arc::new(BasicAuthURIMiddleware::new(
			api_basic_auth,
			&GRIN_FOREIGN_BASIC_REALM,
			"/v2/foreign".into(),
		));
		router.add_middleware(basic_auth_middleware);
	}

	let api_handler = ForeignAPIHandlerV2::new(
		Arc::downgrade(&chain),
		Arc::downgrade(&tx_pool),
		Arc::downgrade(&sync_state),
	);
	router.add_route("/v2/foreign", Arc::new(api_handler))?;

	let mut apis = ApiServer::new();
	warn!("Starting HTTP Node APIs server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread = apis.start(socket_addr, router, tls_config, api_chan);

	warn!("HTTP Node listener started.");

	thread::Builder::new()
		.name("api_monitor".to_string())
		.spawn(move || {
			// monitor for stop state is_stopped
			loop {
				std::thread::sleep(std::time::Duration::from_millis(100));
				if stop_state.is_stopped() {
					apis.stop();
					break;
				}
			}
		})
		.ok();

	match api_thread {
		Ok(_) => Ok(()),
		Err(e) => {
			error!("HTTP API server failed to start. Err: {}", e);
			Err(e)
		}
	}
}

/// V2 API Handler/Wrapper for owner functions
pub struct OwnerAPIHandlerV2 {
	pub chain: Weak<Chain>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl OwnerAPIHandlerV2 {
	/// Create a new owner API handler for GET methods
	pub fn new(chain: Weak<Chain>, peers: Weak<p2p::Peers>, sync_state: Weak<SyncState>) -> Self {
		OwnerAPIHandlerV2 {
			chain,
			peers,
			sync_state,
		}
	}
}

impl crate::router::Handler for OwnerAPIHandlerV2 {
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let api = Owner::new(
			self.chain.clone(),
			self.peers.clone(),
			self.sync_state.clone(),
		);

		Box::pin(async move {
			match parse_body(req).await {
				Ok(val) => {
					let owner_api = &api as &dyn OwnerRpc;
					let res = match owner_api.handle_request(val) {
						MaybeReply::Reply(r) => r,
						MaybeReply::DontReply => {
							// Since it's http, we need to return something. We return [] because jsonrpc
							// clients will parse it as an empty batch response.
							serde_json::json!([])
						}
					};
					Ok(json_response_pretty(&res))
				}
				Err(e) => {
					error!("Request Error: {:?}", e);
					Ok(create_error_response(e))
				}
			}
		})
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::pin(async { Ok(create_ok_response("{}")) })
	}
}

/// V2 API Handler/Wrapper for foreign functions
pub struct ForeignAPIHandlerV2<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	pub chain: Weak<Chain>,
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
	pub sync_state: Weak<SyncState>,
}

impl<B, P> ForeignAPIHandlerV2<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	/// Create a new foreign API handler for GET methods
	pub fn new(
		chain: Weak<Chain>,
		tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
		sync_state: Weak<SyncState>,
	) -> Self {
		ForeignAPIHandlerV2 {
			chain,
			tx_pool,
			sync_state,
		}
	}
}

impl<B, P> crate::router::Handler for ForeignAPIHandlerV2<B, P>
where
	B: BlockChain + 'static,
	P: PoolAdapter + 'static,
{
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let api = Foreign::new(
			self.chain.clone(),
			self.tx_pool.clone(),
			self.sync_state.clone(),
		);

		Box::pin(async move {
			match parse_body(req).await {
				Ok(val) => {
					let foreign_api = &api as &dyn ForeignRpc;
					let res = match foreign_api.handle_request(val) {
						MaybeReply::Reply(r) => r,
						MaybeReply::DontReply => {
							// Since it's http, we need to return something. We return [] because jsonrpc
							// clients will parse it as an empty batch response.
							serde_json::json!([])
						}
					};
					Ok(json_response_pretty(&res))
				}
				Err(e) => {
					error!("Request Error: {:?}", e);
					Ok(create_error_response(e))
				}
			}
		})
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::pin(async { Ok(create_ok_response("{}")) })
	}
}

// pretty-printed version of above
fn json_response_pretty<T>(s: &T) -> Response<Body>
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

fn create_error_response(e: Error) -> Response<Body> {
	Response::builder()
		.status(StatusCode::INTERNAL_SERVER_ERROR)
		.header("access-control-allow-origin", "*")
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		)
		.body(format!("{}", e).into())
		.unwrap()
}

fn create_ok_response(json: &str) -> Response<Body> {
	Response::builder()
		.status(StatusCode::OK)
		.header("access-control-allow-origin", "*")
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		)
		.header(hyper::header::CONTENT_TYPE, "application/json")
		.body(json.to_string().into())
		.unwrap()
}

/// Build a new hyper Response with the status code and body provided.
///
/// Whenever the status code is `StatusCode::OK` the text parameter should be
/// valid JSON as the content type header will be set to `application/json'
fn response<T: Into<Body>>(status: StatusCode, text: T) -> Response<Body> {
	let mut builder = Response::builder();

	builder = builder
		.status(status)
		.header("access-control-allow-origin", "*")
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		);

	if status == StatusCode::OK {
		builder = builder.header(hyper::header::CONTENT_TYPE, "application/json");
	}

	builder.body(text.into()).unwrap()
}
