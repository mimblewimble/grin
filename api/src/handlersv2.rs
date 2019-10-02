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
use crate::rest::{ApiServer, Error, ErrorKind, TLSConfig};
use crate::router::{Handler, ResponseFuture, Router};
use crate::util::to_base64;
use futures::future::ok;
use futures::Future;
use hyper::{Body, Request, Response, StatusCode};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn node_listener(
	addr: &str,
	api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
) -> Result<(), Error> {
	let mut router = Router::new();
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

	let api_handler_v2 = NodeAPIHandlerV2::new();
	//let empty_handler = EmptyHandler {};

	router.add_route("/v2", Arc::new(api_handler_v2))?;

	let mut apis = ApiServer::new();
	warn!("Starting HTTP Node API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread = apis.start(socket_addr, router, tls_config)?;

	warn!("HTTP Node listener started.");
	api_thread
		.join()
		.map_err(|e| ErrorKind::Internal(format!("API thread panicked :{:?}", e)).into())
}

type NodeResponseFuture = Box<dyn Future<Item = Response<Body>, Error = Error> + Send>;

/// V2 API Handler/Wrapper for owner functions
pub struct NodeAPIHandlerV2 {}

impl NodeAPIHandlerV2 {
	/// Create a new owner API handler for GET methods
	pub fn new() -> NodeAPIHandlerV2 {
		NodeAPIHandlerV2 {}
	}
	// UNFINISHED
	fn call_api(
		&self,
		req: Request<Body>,
		//api: Owner<'static, L, C, K>,
	) -> Box<dyn Future<Item = serde_json::Value, Error = Error> + Send> {
		/*Box::new(parse_body(req).and_then(move |val: serde_json::Value| {
			let owner_api = &api as &dyn OwnerRpc;
			match owner_api.handle_request(val) {
				MaybeReply::Reply(r) => ok(r),
				MaybeReply::DontReply => {
					// Since it's http, we need to return something. We return [] because jsonrpc
					// clients will parse it as an empty batch response.
					ok(serde_json::json!([]))
				}
			}
		}))*/
		Box::new(ok(serde_json::json!([])))
	}

	// UNFINISHED
	fn handle_post_request(&self, req: Request<Body>) -> NodeResponseFuture {
		/*let api = Owner::new(self.wallet.clone());
		Box::new(
			self.call_api(req, api)
				.and_then(|resp| ok(json_response_pretty(&resp))),
		)
		*/
		Box::new(
			self.call_api(req)
				.and_then(|resp| ok(json_response_pretty(&resp))),
		)
	}
}

impl Handler for NodeAPIHandlerV2 {
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
