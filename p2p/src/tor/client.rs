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
use crate::tor::config::{TorConfig, REQUEST_TIMEOUT_SECS};
use crate::tor::process::types::TorProcess;
use crate::tor::{Error, PostError};

use bytes::Bytes;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Body, Incoming};
use hyper::http::{HeaderName, HeaderValue};
use hyper::{Request, Response};
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::de::StdError;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::timeout;

/// Tor powered HTTP client to send requests.
pub trait TorHttpClient<T>
where
	T: TorConfig,
{
	fn tor_config(&self) -> &T;

	fn tor_process(&self) -> Option<&TorProcess>;

	fn post(
		&self,
		url: &str,
		headers: Option<HashMap<&str, &str>>,
		body: String,
	) -> Result<String, Error> {
		if self.tor_process().is_some() {
			let mut req: Request<Full<Bytes>> = Request::builder()
				.method(hyper::Method::POST)
				.uri(url)
				.body(Full::from(body))
				.map_err(|e| Error::Generic(format!("Unable to create Tor POST request {}", e)))?;
			if let Some(headers) = headers {
				for (name, value) in headers {
					let header_name = HeaderName::from_str(name).map_err(|e| {
						Error::Generic(format!("Can not parse header name: {}", e.to_string()))
					})?;
					let header_value = HeaderValue::from_str(value).map_err(|e| {
						Error::Generic(format!("Can not parse header value: {}", e.to_string()))
					})?;
					req.headers_mut().insert(header_name, header_value);
				}
			}
			tokio::runtime::Builder::new_current_thread()
				.enable_all()
				.build()
				.unwrap()
				.block_on(async {
					let res = send_socks_proxy(self.tor_config(), req).await?;
					let resp_body = res
						.into_body()
						.collect()
						.await
						.map_err(|e| Error::Generic(e.to_string()))?;
					let body_bytes = resp_body.to_bytes().into();
					let body =
						String::from_utf8(body_bytes).map_err(|e| Error::Generic(e.to_string()))?;
					Ok(body)
				})
		} else {
			tor_post(self.tor_config(), body, headers, url).map_err(|e| Error::Post(e))
		}
	}
}

/// Send request over proxy.
async fn send_socks_proxy<T, B>(config: &T, req: Request<B>) -> Result<Response<Incoming>, Error>
where
	T: TorConfig,
	B: Body + Send + 'static + Unpin,
	<B as Body>::Data: Send,
	B::Data: Send,
	B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
	let connector = HttpsConnector::new();
	let uri = config
		.socks_proxy_addr()
		.parse()
		.map_err(|e| Error::Config(format!("{:?}", e)))?;
	let proxy = hyper_socks2::SocksConnector {
		proxy_addr: uri,
		auth: None,
		connector,
	}
	.with_tls()
	.map_err(|err| Error::Generic(format!("Unable to create socks connector: {}", err)))?;
	let client = Client::builder(TokioExecutor::new()).build::<_, B>(proxy);
	let req_timeout = Duration::from_secs(
		config
			.request_timeout_secs()
			.filter(|s| *s > 0)
			.unwrap_or(REQUEST_TIMEOUT_SECS),
	);
	timeout(req_timeout, async {
		client.request(req).await.map_err(|e| {
			Error::Post(PostError::NotSent(format!(
				"Unable to send request over socks proxy: {}",
				e
			)))
		})
	})
	.await
	.unwrap_or_else(|_| {
		Err(Error::Post(PostError::NotSent(
			"Post request over socks proxy timed out".to_string(),
		)))
	})
}
