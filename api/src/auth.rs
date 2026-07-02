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

use crate::router::{Handler, HandlerObj, ResponseFuture};
use crate::web::response;
use bytes::Bytes;
use futures::future::ok;
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, AUTHORIZATION, WWW_AUTHENTICATE};
use hyper::{Request, Response, StatusCode};
use subtle::ConstantTimeEq;

lazy_static! {
	pub static ref GRIN_BASIC_REALM: HeaderValue =
		HeaderValue::from_str("Basic realm=GrinAPI").unwrap();
	pub static ref GRIN_FOREIGN_BASIC_REALM: HeaderValue =
		HeaderValue::from_str("Basic realm=GrinForeignAPI").unwrap();
}

/// Basic Authentication Middleware
pub struct BasicAuthMiddleware {
	api_basic_auth: String,
	basic_realm: &'static HeaderValue,
	ignore_uri: Option<String>,
}

impl BasicAuthMiddleware {
	pub fn new(
		api_basic_auth: String,
		basic_realm: &'static HeaderValue,
		ignore_uri: Option<String>,
	) -> BasicAuthMiddleware {
		BasicAuthMiddleware {
			api_basic_auth,
			basic_realm,
			ignore_uri,
		}
	}
}

impl Handler for BasicAuthMiddleware {
	fn call(
		&self,
		req: Request<Incoming>,
		mut handlers: Box<dyn Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		let next_handler = match handlers.next() {
			Some(h) => h,
			None => return response(StatusCode::INTERNAL_SERVER_ERROR, "no handler found".into()),
		};
		if req.method().as_str() == "OPTIONS" {
			return next_handler.call(req, handlers);
		}
		if let Some(u) = self.ignore_uri.as_ref() {
			if req.uri().path() == u {
				return next_handler.call(req, handlers);
			}
		}
		if check_auth(&req, &self.api_basic_auth) {
			next_handler.call(req, handlers)
		} else {
			// Unauthorized 401
			unauthorized_response(&self.basic_realm)
		}
	}
}

// Basic Authentication Middleware
pub struct BasicAuthURIMiddleware {
	api_basic_auth: String,
	basic_realm: &'static HeaderValue,
	target_uri: String,
}

impl BasicAuthURIMiddleware {
	pub fn new(
		api_basic_auth: String,
		basic_realm: &'static HeaderValue,
		target_uri: String,
	) -> BasicAuthURIMiddleware {
		BasicAuthURIMiddleware {
			api_basic_auth,
			basic_realm,
			target_uri,
		}
	}
}

impl Handler for BasicAuthURIMiddleware {
	fn call(
		&self,
		req: Request<Incoming>,
		mut handlers: Box<dyn Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		let next_handler = match handlers.next() {
			Some(h) => h,
			None => return response(StatusCode::INTERNAL_SERVER_ERROR, "no handler found".into()),
		};
		if req.method().as_str() == "OPTIONS" {
			return next_handler.call(req, handlers);
		}
		if req.uri().path() == self.target_uri {
			if check_auth(&req, &self.api_basic_auth) {
				next_handler.call(req, handlers)
			} else {
				// Unauthorized 401
				unauthorized_response(&self.basic_realm)
			}
		} else {
			next_handler.call(req, handlers)
		}
	}
}

fn check_auth(req: &Request<Incoming>, api_basic_auth: &String) -> bool {
	if req.headers().contains_key(AUTHORIZATION) {
		let req_auth = req.headers()[AUTHORIZATION].as_bytes();
		let auth = api_basic_auth.as_bytes();
		return req_auth.ct_eq(auth).unwrap_u8() == 1;
	}
	false
}

fn unauthorized_response(basic_realm: &HeaderValue) -> ResponseFuture {
	let response = Response::builder()
		.status(StatusCode::UNAUTHORIZED)
		.header(WWW_AUTHENTICATE, basic_realm)
		.body(Empty::<Bytes>::new().boxed())
		.unwrap();
	Box::pin(ok(response))
}
