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

use std::cell::RefCell;
use std::fmt::Debug;

use futures::future::{err, ok};
use futures::{Future, Stream};
use hyper::{Body, Request, Response, StatusCode};
use rest::{Error, ErrorKind};
use serde::{Deserialize, Serialize};
use serde_json;

use router::{ResponseFuture, Router};
use util::LOGGER;

// Utility to serialize a struct into JSON and produce a sensible Response
// out of it.
pub fn json_response<T>(s: &T) -> ResponseFuture
where
	T: Serialize,
{
	match serde_json::to_string(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

pub fn result_to_response<T>(res: Result<T, Error>) -> ResponseFuture
where
	T: Serialize,
{
	match res {
		Ok(s) => json_response_pretty(&s),
		Err(e) => match e.kind() {
			ErrorKind::Argument(msg) => response(StatusCode::BAD_REQUEST, msg.clone()),
			ErrorKind::RequestError(msg) => response(StatusCode::BAD_REQUEST, msg.clone()),
			ErrorKind::NotFound => response(StatusCode::NOT_FOUND, ""),
			ErrorKind::Internal(msg) => response(StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
			ErrorKind::ResponseError(msg) => {
				response(StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
			}
		},
	}
}

// pretty-printed version of above
pub fn json_response_pretty<T>(s: &T) -> ResponseFuture
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(e) => response(
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("can't create json response: {}", e),
		),
	}
}

pub fn response<T: Into<Body> + Debug>(status: StatusCode, text: T) -> ResponseFuture {
	Box::new(ok(just_response(status, text)))
}

pub fn just_response<T: Into<Body> + Debug>(status: StatusCode, text: T) -> Response<Body> {
	let mut resp = Response::new(text.into());
	*resp.status_mut() = status;
	resp
}

thread_local!( pub static ROUTER: RefCell<Option<Router>> = RefCell::new(None) );

pub fn handle(req: Request<Body>) -> ResponseFuture {
	ROUTER.with(|router| match *router.borrow() {
		Some(ref h) => h.handle(req),
		None => {
			error!(LOGGER, "No HTTP API router configured");
			response(StatusCode::INTERNAL_SERVER_ERROR, "No router configured")
		}
	})
}

pub fn parse_body<T>(req: Request<Body>) -> Box<Future<Item = T, Error = Error> + Send>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	Box::new(
		req.into_body()
			.concat2()
			.map_err(|_e| ErrorKind::RequestError("Failed to read request".to_owned()).into())
			.and_then(|body| match serde_json::from_reader(&body.to_vec()[..]) {
				Ok(obj) => ok(obj),
				Err(_) => err(ErrorKind::RequestError("Invalid request body".to_owned()).into()),
			}),
	)
}
