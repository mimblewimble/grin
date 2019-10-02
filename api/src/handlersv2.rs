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

//! Handler for Node RPC v2 API, instantiates and handles listener.
use crate::auth::{BasicAuthMiddleware, GRIN_BASIC_REALM};
use crate::chain;
use crate::chain::{Chain, SyncState};
use crate::handlers::build_router;
use crate::node::Node;
use crate::node_rpc::NodeRpc;
use crate::p2p;
use crate::pool;
use crate::rest::{ApiServer, Error, TLSConfig};
use crate::router::ResponseFuture;
use crate::util::to_base64;
use crate::util::RwLock;
use crate::web::*;
use easy_jsonrpc_mw::{Handler, MaybeReply};
use futures::future::ok;
use futures::Future;
use hyper::{Body, Request, Response, StatusCode};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Weak;

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn node_api(
	addr: &str,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	peers: Arc<p2p::Peers>,
	sync_state: Arc<chain::SyncState>,
	api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
) -> Result<(), Error> {
	// Manually build router when getting rid of v1
	//let mut router = Router::new();
	let mut router = build_router(
		chain.clone(),
		tx_pool.clone(),
		peers.clone(),
		sync_state.clone(),
	)
	.expect("unable to build API router");

	if let Some(api_secret) = api_secret {
		let api_basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret));
		let basic_auth_middleware = Arc::new(BasicAuthMiddleware::new(
			api_basic_auth,
			&GRIN_BASIC_REALM,
			Some("/v2".into()),
		));
		router.add_middleware(basic_auth_middleware);
	}

	let api_handler_v2 = NodeAPIHandlerV2::new(
		Arc::downgrade(&chain),
		Arc::downgrade(&peers),
		Arc::downgrade(&sync_state),
	);

	router.add_route("/v2", Arc::new(api_handler_v2))?;

	let mut apis = ApiServer::new();
	warn!("Starting HTTP Node API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread = apis.start(socket_addr, router, tls_config);

	warn!("HTTP Node listener started.");

	// FIX THIS: Do not lock here when using the command below
	/*api_thread
	.join()
	.map_err(|e| ErrorKind::Internal(format!("API thread panicked :{:?}", e)).into())
	*/
	match api_thread {
		Ok(_) => Ok(()),
		Err(e) => {
			error!("HTTP API server failed to start. Err: {}", e);
			Err(e)
		}
	}
}

type NodeResponseFuture = Box<dyn Future<Item = Response<Body>, Error = Error> + Send>;

/// V2 API Handler/Wrapper for owner functions
pub struct NodeAPIHandlerV2 {
	pub chain: Weak<Chain>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl NodeAPIHandlerV2 {
	/// Create a new owner API handler for GET methods
	pub fn new(chain: Weak<Chain>, peers: Weak<p2p::Peers>, sync_state: Weak<SyncState>) -> Self {
		NodeAPIHandlerV2 {
			chain,
			peers,
			sync_state,
		}
	}

	fn call_api(
		&self,
		req: Request<Body>,
		api: Node,
	) -> Box<dyn Future<Item = serde_json::Value, Error = Error> + Send> {
		Box::new(parse_body(req).and_then(move |val: serde_json::Value| {
			let node_api = &api as &dyn NodeRpc;
			match node_api.handle_request(val) {
				MaybeReply::Reply(r) => ok(r),
				MaybeReply::DontReply => {
					// Since it's http, we need to return something. We return [] because jsonrpc
					// clients will parse it as an empty batch response.
					ok(serde_json::json!([]))
				}
			}
		}))
	}

	// UNFINISHED
	fn handle_post_request(&self, req: Request<Body>) -> NodeResponseFuture {
		let api = Node::new(
			self.chain.clone(),
			self.peers.clone(),
			self.sync_state.clone(),
		);
		Box::new(
			self.call_api(req, api)
				.and_then(|resp| ok(json_response_pretty(&resp))),
		)
	}
}

impl crate::router::Handler for NodeAPIHandlerV2 {
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(
			self.handle_post_request(req)
				.and_then(|r| ok(r))
				.or_else(|e| {
					error!("Request Error: {:?}", e);
					ok(create_error_response(e))
				}),
		)
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::new(ok(create_ok_response("{}")))
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
	let mut builder = &mut Response::builder();

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
