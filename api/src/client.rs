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

//! High level JSON/HTTP client API

use failure::{Fail, ResultExt};
use http::uri::{InvalidUri, Uri};
use hyper::header::{ACCEPT, USER_AGENT};
use hyper::rt::{Future, Stream};
use hyper::{Body, Client, Request};
use serde::{Deserialize, Serialize};
use serde_json;

use futures::future::{err, ok, Either};
use tokio_core::reactor::Core;

use rest::{Error, ErrorKind};

/// Helper function to easily issue a HTTP GET request against a given URL that
/// returns a JSON object. Handles request building, JSON deserialization and
/// response code checking.
pub fn get<'a, T>(url: &'a str) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	let uri = url.parse::<Uri>().map_err::<Error, _>(|e: InvalidUri| {
		e.context(ErrorKind::Argument(format!("Invalid url {}", url)))
			.into()
	})?;
	let req = Request::builder()
		.method("GET")
		.uri(uri)
		.header(USER_AGENT, "grin-client")
		.header(ACCEPT, "application/json")
		.body(Body::empty())
		.map_err(|_e| ErrorKind::RequestError("Bad request".to_owned()))?;

	handle_request(req)
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns a JSON object. Handles request
/// building, JSON serialization and deserialization, and response code
/// checking.
pub fn post<IN, OUT>(url: &str, input: &IN) -> Result<OUT, Error>
where
	IN: Serialize,
	for<'de> OUT: Deserialize<'de>,
{
	let req = create_post_request(url, input)?;
	handle_request(req)
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns nothing. Handles request
/// building, JSON serialization, and response code
/// checking.
pub fn post_no_ret<IN>(url: &str, input: &IN) -> Result<(), Error>
where
	IN: Serialize,
{
	let req = create_post_request(url, input)?;
	send_request(req)?;
	Ok(())
}

fn create_post_request<IN>(url: &str, input: &IN) -> Result<Request<Body>, Error>
where
	IN: Serialize,
{
	let json = serde_json::to_string(input).context(ErrorKind::Internal(
		"Could not serialize data to JSON".to_owned(),
	))?;
	let uri = url.parse::<Uri>().map_err::<Error, _>(|e: InvalidUri| {
		e.context(ErrorKind::Argument(format!("Invalid url {}", url)))
			.into()
	})?;
	Request::builder()
		.method("POST")
		.uri(uri)
		.header(USER_AGENT, "grin-client")
		.header(ACCEPT, "application/json")
		.body(json.into())
		.map_err::<Error, _>(|e| {
			e.context(ErrorKind::RequestError("Bad request".to_owned()))
				.into()
		})
}

fn handle_request<T>(req: Request<Body>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	let data = send_request(req)?;
	serde_json::from_str(&data).map_err(|e| {
		e.context(ErrorKind::ResponseError("Cannot parse response".to_owned()))
			.into()
	})
}

fn send_request(req: Request<Body>) -> Result<String, Error> {
	let mut event_loop = Core::new().unwrap();
	let client = Client::new();

	let task = client
		.request(req)
		.map_err(|_e| ErrorKind::RequestError("Cannot make request".to_owned()))
		.and_then(|resp| {
			if !resp.status().is_success() {
				Either::A(err(ErrorKind::RequestError(
					"Wrong response code".to_owned(),
				)))
			} else {
				Either::B(
					resp.into_body()
						.map_err(|_e| {
							ErrorKind::RequestError("Cannot read response body".to_owned())
						})
						.concat2()
						.and_then(|ch| ok(String::from_utf8_lossy(&ch.to_vec()).to_string())),
				)
			}
		});

	Ok(event_loop.run(task)?)
}
