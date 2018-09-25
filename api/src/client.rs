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
use hyper::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use hyper::rt::{Future, Stream};
use hyper::{Body, Client, Request};
use hyper_tls;
use serde::{Deserialize, Serialize};
use serde_json;

use futures::future::{err, ok, Either};
use tokio::runtime::Runtime;

use rest::{Error, ErrorKind};
use util::to_base64;

pub type ClientResponseFuture<T> = Box<Future<Item = T, Error = Error> + Send>;

/// Helper function to easily issue a HTTP GET request against a given URL that
/// returns a JSON object. Handles request building, JSON deserialization and
/// response code checking.
pub fn get<'a, T>(url: &'a str, api_secret: Option<String>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	handle_request(build_request(url, "GET", api_secret, None)?)
}

/// Helper function to easily issue an async HTTP GET request against a given
/// URL that returns a future. Handles request building, JSON deserialization
/// and response code checking.
pub fn get_async<'a, T>(url: &'a str, api_secret: Option<String>) -> ClientResponseFuture<T>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	match build_request(url, "GET", api_secret, None) {
		Ok(req) => Box::new(handle_request_async(req)),
		Err(e) => Box::new(err(e)),
	}
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns a JSON object. Handles request
/// building, JSON serialization and deserialization, and response code
/// checking.
pub fn post<IN, OUT>(url: &str, api_secret: Option<String>, input: &IN) -> Result<OUT, Error>
where
	IN: Serialize,
	for<'de> OUT: Deserialize<'de>,
{
	let req = create_post_request(url, api_secret, input)?;
	handle_request(req)
}

/// Helper function to easily issue an async HTTP POST request with the
/// provided JSON object as body on a given URL that returns a future. Handles
/// request building, JSON serialization and deserialization, and response code
/// checking.
pub fn post_async<IN, OUT>(
	url: &str,
	input: &IN,
	api_secret: Option<String>,
) -> ClientResponseFuture<OUT>
where
	IN: Serialize,
	OUT: Send + 'static,
	for<'de> OUT: Deserialize<'de>,
{
	match create_post_request(url, api_secret, input) {
		Ok(req) => Box::new(handle_request_async(req)),
		Err(e) => Box::new(err(e)),
	}
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns nothing. Handles request
/// building, JSON serialization, and response code
/// checking.
pub fn post_no_ret<IN>(url: &str, api_secret: Option<String>, input: &IN) -> Result<(), Error>
where
	IN: Serialize,
{
	let req = create_post_request(url, api_secret, input)?;
	send_request(req)?;
	Ok(())
}

/// Helper function to easily issue an async HTTP POST request with the
/// provided JSON object as body on a given URL that returns a future. Handles
/// request building, JSON serialization and deserialization, and response code
/// checking.
pub fn post_no_ret_async<IN>(
	url: &str,
	api_secret: Option<String>,
	input: &IN,
) -> ClientResponseFuture<()>
where
	IN: Serialize,
{
	match create_post_request(url, api_secret, input) {
		Ok(req) => Box::new(send_request_async(req).and_then(|_| ok(()))),
		Err(e) => Box::new(err(e)),
	}
}

fn build_request<'a>(
	url: &'a str,
	method: &str,
	api_secret: Option<String>,
	body: Option<String>,
) -> Result<Request<Body>, Error> {
	let uri = url.parse::<Uri>().map_err::<Error, _>(|e: InvalidUri| {
		e.context(ErrorKind::Argument(format!("Invalid url {}", url)))
			.into()
	})?;
	let mut builder = Request::builder();
	if api_secret.is_some() {
		let basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret.unwrap()));
		builder.header(AUTHORIZATION, basic_auth);
	}

	builder
		.method(method)
		.uri(uri)
		.header(USER_AGENT, "grin-client")
		.header(ACCEPT, "application/json")
		.body(match body {
			None => Body::empty(),
			Some(json) => json.into(),
		}).map_err(|e| {
			ErrorKind::RequestError(format!("Bad request {} {}: {}", method, url, e)).into()
		})
}

fn create_post_request<IN>(
	url: &str,
	api_secret: Option<String>,
	input: &IN,
) -> Result<Request<Body>, Error>
where
	IN: Serialize,
{
	let json = serde_json::to_string(input).context(ErrorKind::Internal(
		"Could not serialize data to JSON".to_owned(),
	))?;
	build_request(url, "POST", api_secret, Some(json))
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

fn handle_request_async<T>(req: Request<Body>) -> ClientResponseFuture<T>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	Box::new(send_request_async(req).and_then(|data| {
		serde_json::from_str(&data).map_err(|e| {
			e.context(ErrorKind::ResponseError("Cannot parse response".to_owned()))
				.into()
		})
	}))
}

fn send_request_async(req: Request<Body>) -> Box<Future<Item = String, Error = Error> + Send> {
	let https = hyper_tls::HttpsConnector::new(1).unwrap();
	let client = Client::builder().build::<_, Body>(https);
	Box::new(
		client
			.request(req)
			.map_err(|e| ErrorKind::RequestError(format!("Cannot make request: {}", e)).into())
			.and_then(|resp| {
				if !resp.status().is_success() {
					Either::A(err(ErrorKind::RequestError(
						"Wrong response code".to_owned(),
					).into()))
				} else {
					Either::B(
						resp.into_body()
							.map_err(|e| {
								ErrorKind::RequestError(format!("Cannot read response body: {}", e))
									.into()
							}).concat2()
							.and_then(|ch| ok(String::from_utf8_lossy(&ch.to_vec()).to_string())),
					)
				}
			}),
	)
}

fn send_request(req: Request<Body>) -> Result<String, Error> {
	let task = send_request_async(req);
	let mut rt = Runtime::new().unwrap();
	Ok(rt.block_on(task)?)
}
