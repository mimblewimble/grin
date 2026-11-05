// Copyright 2026 The Grin Developers
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

use crate::tor::arti::tor_post;
use crate::tor::config::{request_timeout, TorConfig};
use crate::tor::{Error, PostError};

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Body;
use hyper::{Request, Response};
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::de::StdError;
use std::collections::HashMap;
use tokio::time::timeout;

/// Tor powered HTTP client to send requests.
#[async_trait]
pub trait TorHttpClient<T>: Send + Sync
where
	T: TorConfig + Sync,
{
	fn tor_config(&self) -> &T;

	/// POST with a response body limit in bytes
	/// Request timeout includes client startup, connection and body
	async fn post_tor(
		&self,
		url: &str,
		headers: Option<HashMap<&str, &str>>,
		body: String,
		response_limit: usize,
	) -> Result<Response<String>, Error> {
		if !self.tor_config().use_integrated() {
			let mut req = Request::builder().method("POST").uri(url);
			if let Some(headers) = headers {
				for (name, value) in headers {
					req = req.header(name, value);
				}
			}
			let req = req.body(Full::new(Bytes::from(body))).map_err(|e| {
				Error::Post(PostError::NotSent(format!("Cannot build request: {}", e)))
			})?;
			send_socks_proxy(self.tor_config(), req, response_limit).await
		} else {
			tor_post(self.tor_config(), body, headers, url, response_limit)
				.await
				.map_err(Error::Post)
		}
	}
}

/// Send request over proxy.
async fn send_socks_proxy<T, B>(
	config: &T,
	req: Request<B>,
	response_limit: usize,
) -> Result<Response<String>, Error>
where
	T: TorConfig,
	B: Body + Send + 'static + Unpin,
	<B as Body>::Data: Send,
	B::Data: Send,
	B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	let mut addr = config.socks_proxy_addr();
	let socks_prefix = "socks5://";
	if addr.starts_with("http://") {
		addr = addr.replace("http://", socks_prefix);
	} else if addr.starts_with("https://") {
		addr = addr.replace("https://", socks_prefix);
	} else if !addr.starts_with(socks_prefix) {
		addr = format!("{}{}", socks_prefix, addr);
	}

	let connector = HttpsConnector::new();
	let uri = addr
		.parse()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	let proxy = hyper_socks2::SocksConnector {
		proxy_addr: uri,
		auth: None,
		connector,
	}
	.with_tls()
	.map_err(|err| Error::Generic(format!("Unable to create socks connector: {}", err)))?;

	debug!("socks_proxy: {:?}", proxy);

	let client = Client::builder(TokioExecutor::new())
		.retry_canceled_requests(false)
		.build::<_, B>(proxy);
	let mut status = None;
	timeout(request_timeout(config), async {
		let response = client.request(req).await.map_err(|e| {
			let message = format!("Unable to send request over socks proxy: {:?}", e);
			if e.is_connect() {
				PostError::NotSent(message)
			} else {
				PostError::PossiblySent(message, None, None)
			}
		})?;
		status = Some(response.status());
		read_response(response, response_limit).await
	})
	.await
	.unwrap_or_else(|_| {
		Err(PostError::PossiblySent(
			"Post request over socks proxy timed out".to_string(),
			status,
			None,
		))
	})
	.map_err(Error::Post)
}

pub(crate) async fn read_response<B>(
	response: Response<B>,
	limit: usize,
) -> Result<Response<String>, PostError>
where
	B: Body,
	B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	let (parts, body) = response.into_parts();
	let status = parts.status;
	let body = Limited::new(body, limit)
		.collect()
		.await
		.map_err(|e| PostError::PossiblySent(e.to_string(), Some(status), None))?;
	let body = String::from_utf8(body.to_bytes().to_vec())
		.map_err(|e| PostError::PossiblySent(e.to_string(), Some(status), None))?;
	if !status.is_success() {
		return Err(PostError::PossiblySent(
			format!("HTTP request failed with status {}", status),
			Some(status),
			Some(body),
		));
	}
	Ok(Response::from_parts(parts, body))
}

#[cfg(test)]
mod tests {
	use super::*;
	use futures::stream;
	use http_body_util::StreamBody;
	use hyper::body::Frame;
	use hyper::StatusCode;
	use std::io;

	#[tokio::test]
	async fn response_status() {
		let response = Response::builder()
			.status(StatusCode::CREATED)
			.header("location", "/result")
			.body(Full::<Bytes>::from("done"))
			.unwrap();
		let response = read_response(response, 4).await.unwrap();
		assert_eq!(response.status(), StatusCode::CREATED);
		assert_eq!(response.headers()["location"], "/result");
		assert_eq!(response.body(), "done");

		let response = Response::builder()
			.status(StatusCode::NO_CONTENT)
			.body(Full::<Bytes>::default())
			.unwrap();
		let response = read_response(response, 0).await.unwrap();
		assert_eq!(response.status(), StatusCode::NO_CONTENT);
		assert!(response.body().is_empty());

		let response = Response::builder()
			.status(StatusCode::TOO_MANY_REQUESTS)
			.body(Full::<Bytes>::from("retry later"))
			.unwrap();
		assert!(matches!(read_response(response, 11).await,
			Err(PostError::PossiblySent(_, Some(StatusCode::TOO_MANY_REQUESTS), Some(body)))
			if body == "retry later"));
	}

	#[tokio::test]
	async fn body_limit() {
		// No Content-Length
		let body = StreamBody::new(stream::iter([
			Ok::<_, io::Error>(Frame::data(Bytes::from_static(b"ab"))),
			Ok(Frame::data(Bytes::from_static(b"cd"))),
		]));
		let response = Response::builder()
			.status(StatusCode::SERVICE_UNAVAILABLE)
			.body(body)
			.unwrap();
		assert!(matches!(
			read_response(response, 3).await,
			Err(PostError::PossiblySent(
				_,
				Some(StatusCode::SERVICE_UNAVAILABLE),
				None
			))
		));
	}

	#[tokio::test]
	async fn body_error() {
		let body = StreamBody::new(stream::iter([
			Ok(Frame::data(Bytes::from_static(b"partial"))),
			Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
		]));
		assert!(matches!(
			read_response(Response::new(body), 100).await,
			Err(PostError::PossiblySent(_, Some(StatusCode::OK), None))
		));

		let body = Full::new(Bytes::from_static(&[0xff]));
		assert!(matches!(
			read_response(Response::new(body), 100).await,
			Err(PostError::PossiblySent(_, Some(StatusCode::OK), None))
		));
	}
}
