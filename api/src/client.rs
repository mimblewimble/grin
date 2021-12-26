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

//! High level JSON/HTTP client API

use crate::rest::{Error, ErrorKind};
use crate::util::to_base64;
use failure::{Fail, ResultExt};
use hyper::body;
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use hyper::{Body, Client, Request};
use hyper_timeout::TimeoutConnector;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::runtime::Builder;

/// Helper function to easily issue a HTTP GET request against a given URL that
/// returns a JSON object. Handles request building, JSON deserialization and
/// response code checking.
/// This function spawns a new Tokio runtime, which means it is pretty inefficient for multiple
/// requests. In those situations you are probably better off creating a runtime once and spawning
/// `get_async` tasks on it
pub fn get<T>(url: &str, api_secret: Option<String>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	handle_request(build_request(url, "GET", api_secret, None)?, None)
}

/// Helper function to easily issue an async HTTP GET request against a given
/// URL that returns a future. Handles request building, JSON deserialization
/// and response code checking.
pub async fn get_async<T>(url: &str, api_secret: Option<String>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	handle_request_async(build_request(url, "GET", api_secret, None)?).await
}

/// Helper function to easily issue a HTTP GET request
/// on a given URL that returns nothing. Handles request
/// building and response code checking.
pub fn get_no_ret(url: &str, api_secret: Option<String>) -> Result<(), Error> {
	send_request(build_request(url, "GET", api_secret, None)?, None)?;
	Ok(())
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns a JSON object. Handles request
/// building, JSON serialization and deserialization, and response code
/// checking.
pub fn post<IN, OUT>(
	url: &str,
	api_secret: Option<String>,
	input: &IN,
	read_timeout: Option<u64>,
) -> Result<OUT, Error>
where
	IN: Serialize,
	for<'de> OUT: Deserialize<'de>,
{
	let req = create_post_request(url, api_secret, input)?;
	handle_request(req, read_timeout)
}

/// Helper function to easily issue an async HTTP POST request with the
/// provided JSON object as body on a given URL that returns a future. Handles
/// request building, JSON serialization and deserialization, and response code
/// checking.
pub async fn post_async<IN, OUT>(
	url: &str,
	input: &IN,
	api_secret: Option<String>,
) -> Result<OUT, Error>
where
	IN: Serialize,
	OUT: Send + 'static,
	for<'de> OUT: Deserialize<'de>,
{
	handle_request_async(create_post_request(url, api_secret, input)?).await
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns nothing. Handles request
/// building, JSON serialization, and response code
/// checking.
pub fn post_no_ret<IN>(url: &str, api_secret: Option<String>, input: &IN) -> Result<(), Error>
where
	IN: Serialize,
{
	send_request(create_post_request(url, api_secret, input)?, None)?;
	Ok(())
}

/// Helper function to easily issue an async HTTP POST request with the
/// provided JSON object as body on a given URL that returns a future. Handles
/// request building, JSON serialization and deserialization, and response code
/// checking.
pub async fn post_no_ret_async<IN>(
	url: &str,
	api_secret: Option<String>,
	input: &IN,
) -> Result<(), Error>
where
	IN: Serialize,
{
	send_request_async(create_post_request(url, api_secret, input)?, None).await?;
	Ok(())
}

fn build_request(
	url: &str,
	method: &str,
	api_secret: Option<String>,
	body: Option<String>,
) -> Result<Request<Body>, Error> {
	let mut builder = Request::builder();
	if let Some(api_secret) = api_secret {
		let basic_auth = format!("Basic {}", to_base64(&format!("grin:{}", api_secret)));
		builder = builder.header(AUTHORIZATION, basic_auth);
	}

	builder
		.method(method)
		.uri(url)
		.header(USER_AGENT, "grin-client")
		.header(ACCEPT, "application/json")
		.header(CONTENT_TYPE, "application/json")
		.body(match body {
			None => Body::empty(),
			Some(json) => json.into(),
		})
		.map_err(|e| {
			ErrorKind::RequestError(format!("Bad request {} {}: {}", method, url, e)).into()
		})
}

pub fn create_post_request<IN>(
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

fn handle_request<T>(req: Request<Body>, read_timeout: Option<u64>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	let data = send_request(req, read_timeout)?;
	serde_json::from_str(&data).map_err(|e| {
		e.context(ErrorKind::ResponseError("Cannot parse response".to_owned()))
			.into()
	})
}

async fn handle_request_async<T>(req: Request<Body>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	let data = send_request_async(req, None).await?;
	let ser = serde_json::from_str(&data)
		.map_err(|e| e.context(ErrorKind::ResponseError("Cannot parse response".to_owned())))?;
	Ok(ser)
}

async fn send_request_async(
	req: Request<Body>,
	read_timeout: Option<u64>,
) -> Result<String, Error> {
	let https = hyper_rustls::HttpsConnector::new();
	let read_timeout = match read_timeout {
		Some(v) => Duration::from_secs(v),
		None => Duration::from_secs(20),
	};
	let mut connector = TimeoutConnector::new(https);
	connector.set_connect_timeout(Some(Duration::from_secs(20)));
	connector.set_read_timeout(Some(read_timeout));
	connector.set_write_timeout(Some(Duration::from_secs(20)));
	let client = Client::builder().build::<_, Body>(connector);

	let resp = client
		.request(req)
		.await
		.map_err(|e| ErrorKind::RequestError(format!("Cannot make request: {}", e)))?;

	if !resp.status().is_success() {
		return Err(ErrorKind::RequestError(format!(
			"Wrong response code: {} with data {:?}",
			resp.status(),
			resp.body()
		))
		.into());
	}

	let raw = body::to_bytes(resp)
		.await
		.map_err(|e| ErrorKind::RequestError(format!("Cannot read response body: {}", e)))?;

	Ok(String::from_utf8_lossy(&raw).to_string())
}

pub fn send_request(req: Request<Body>, read_timeout: Option<u64>) -> Result<String, Error> {
	let mut rt = Builder::new()
		.basic_scheduler()
		.enable_all()
		.build()
		.map_err(|e| ErrorKind::RequestError(format!("{}", e)))?;
	rt.block_on(send_request_async(req, read_timeout))
}
