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

use crate::router::{Handler, HandlerObj, ResponseFuture};
use futures::future::ok;
use hyper::header::{HeaderValue, AUTHORIZATION, WWW_AUTHENTICATE};
use hyper::{Body, Request, Response, StatusCode};
use ring::constant_time::verify_slices_are_equal;

// Basic Authentication Middleware
pub struct BasicAuthMiddleware {
	api_basic_auth: String,
	basic_realm: String,
}

impl BasicAuthMiddleware {
	pub fn new(api_basic_auth: String, basic_realm: String) -> BasicAuthMiddleware {
		BasicAuthMiddleware {
			api_basic_auth,
			basic_realm,
		}
	}
}

impl Handler for BasicAuthMiddleware {
	fn call(
		&self,
		req: Request<Body>,
		mut handlers: Box<dyn Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		if req.method().as_str() == "OPTIONS" {
			return handlers.next().unwrap().call(req, handlers);
		}
		if req.headers().contains_key(AUTHORIZATION)
			&& verify_slices_are_equal(
				req.headers()[AUTHORIZATION].as_bytes(),
				&self.api_basic_auth.as_bytes(),
			)
			.is_ok()
		{
			handlers.next().unwrap().call(req, handlers)
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
		)
		.body(Body::empty())
		.unwrap();
	Box::new(ok(response))
}
