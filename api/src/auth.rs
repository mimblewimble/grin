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

use futures::future::ok;
use hyper::header::{HeaderValue, AUTHORIZATION, WWW_AUTHENTICATE};
use hyper::{Body, Request, Response, StatusCode};
use router::{Handler, HandlerObj, ResponseFuture};

// Basic Authentication Middleware
pub struct BasicAuthMiddleware {
	next: HandlerObj,
	api_basic_auth: String,
	basic_realm: String,
}

impl BasicAuthMiddleware {
	pub fn new(next: HandlerObj, api_basic_auth_ref: &str, realm_ref: &str) -> BasicAuthMiddleware {
		let api_basic_auth = api_basic_auth_ref.to_owned();
		let basic_realm = "Basic realm=\"".to_owned() + realm_ref + "\"";
		BasicAuthMiddleware {
			next,
			api_basic_auth,
			basic_realm,
		}
	}
}

impl Handler for BasicAuthMiddleware {
	fn call(&self, req: Request<Body>) -> ResponseFuture {
		if req.headers().contains_key(AUTHORIZATION) {
			if req.headers()[AUTHORIZATION] == self.api_basic_auth {
				self.next.call(req)
			} else {
				// Forbidden 403
				forbidden_response()
			}
		} else {
			// Unauthorized 401
			unauthorized_response(&self.basic_realm)
		}
	}
}

fn unauthorized_response(basic_realm: &str) -> ResponseFuture {
	let response = Response::builder()
		.status(StatusCode::UNAUTHORIZED)
		.header(
			WWW_AUTHENTICATE,
			HeaderValue::from_str(basic_realm).unwrap(),
		).body(Body::empty())
		.unwrap();
	Box::new(ok(response))
}

fn forbidden_response() -> ResponseFuture {
	let response = Response::builder()
		.status(StatusCode::FORBIDDEN)
		.body(Body::empty())
		.unwrap();
	Box::new(ok(response))
}
