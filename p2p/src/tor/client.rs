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

use crate::tor::config::{request_timeout, TorConfig, TorOutboundProxy};
use crate::tor::{Error, PostError};

use arti_client::StreamPrefs;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{future::BoxFuture, FutureExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Body;
use hyper::{Method, Request, Response, StatusCode, Uri};
use hyper_tls::HttpsConnector;
use serde::de::StdError;
use std::collections::HashMap;
use tokio::sync::{watch, Mutex};
use tokio::time::{timeout_at, Instant};
use tower_service::Service;

/// One connection per context, with fixed configuration
/// Release Arti contexts, listeners and requests before changing configuration
pub struct TorContext<T> {
	config: T,
	state: Mutex<ConnectionState>,
	closed: watch::Sender<bool>,
}

impl<T> TorContext<T> {
	pub fn new(config: T) -> Self {
		Self {
			config,
			state: Mutex::new(ConnectionState::default()),
			closed: watch::channel(false).0,
		}
	}

	/// Cancel pending requests and close the cached connection
	/// Further requests require a new context
	pub async fn close(&self) {
		self.closed.send_replace(true);
		*self.state.lock().await = ConnectionState::default();
	}
}

#[derive(Default)]
struct ConnectionState {
	cached: Option<(Uri, HttpConnection)>,
	isolation: Option<(StreamPrefs, Option<String>)>,
}

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
		request_tor(
			self.tor_config(),
			Method::POST,
			url,
			headers,
			body,
			response_limit,
		)
		.await
	}

	/// Send HEAD to check HTTP reachability within the request timeout
	/// HTTP errors also confirm a response
	async fn check_tor(&self, url: &str) -> Result<StatusCode, Error> {
		probe_status(
			request_tor(self.tor_config(), Method::HEAD, url, None, String::new(), 0).await,
		)
	}
}

#[async_trait]
impl<T: TorConfig + Send + Sync> TorHttpClient<T> for TorContext<T> {
	fn tor_config(&self) -> &T {
		&self.config
	}

	async fn post_tor(
		&self,
		url: &str,
		headers: Option<HashMap<&str, &str>>,
		body: String,
		response_limit: usize,
	) -> Result<Response<String>, Error> {
		self.request(Method::POST, url, headers, body, response_limit)
			.await
	}

	async fn check_tor(&self, url: &str) -> Result<StatusCode, Error> {
		probe_status(
			self.request(Method::HEAD, url, None, String::new(), 0)
				.await,
		)
	}
}

impl<T: TorConfig> TorContext<T> {
	async fn request(
		&self,
		method: Method,
		url: &str,
		headers: Option<HashMap<&str, &str>>,
		body: String,
		limit: usize,
	) -> Result<Response<String>, Error> {
		let deadline = Instant::now() + request_timeout(&self.config);
		let (url, request) = build_request(method, body, headers, url).map_err(Error::Post)?;
		let mut closed = self.closed.subscribe();
		if *closed.borrow() {
			return Err(Error::Post(PostError::NotSent("Tor context closed".into())));
		}
		let mut state = tokio::select! {
			biased;
			_ = closed.changed() => return Err(Error::Post(PostError::NotSent("Tor context closed".into()))),
			state = timeout_at(deadline, self.state.lock()) => state.map_err(|_| Error::Post(PostError::NotSent("Tor request timed out".into())))?,
		};
		exchange(
			&self.config,
			&mut state,
			url,
			request,
			limit,
			deadline,
			&mut closed,
		)
		.await
	}
}

fn probe_status(result: Result<Response<String>, Error>) -> Result<StatusCode, Error> {
	match result {
		Ok(response) => Ok(response.status()),
		Err(Error::Post(PostError::PossiblySent(_, Some(status), Some(_)))) => Ok(status),
		Err(error) => Err(error),
	}
}

async fn request_tor<T: TorConfig>(
	config: &T,
	method: Method,
	url: &str,
	headers: Option<HashMap<&str, &str>>,
	body: String,
	response_limit: usize,
) -> Result<Response<String>, Error> {
	let deadline = Instant::now() + request_timeout(config);
	let (url, request) = build_request(method, body, headers, url).map_err(Error::Post)?;
	let (_close, mut closed) = watch::channel(false);
	exchange(
		config,
		&mut ConnectionState {
			isolation: Some((StreamPrefs::default(), None)),
			..Default::default()
		},
		url,
		request,
		response_limit,
		deadline,
		&mut closed,
	)
	.await
}

async fn exchange<T: TorConfig>(
	config: &T,
	state: &mut ConnectionState,
	url: Uri,
	mut request: Request<Full<Bytes>>,
	limit: usize,
	deadline: Instant,
	closed: &mut watch::Receiver<bool>,
) -> Result<Response<String>, Error> {
	let proxy = config.outbound_proxy();
	let proxy_uri = proxy.as_ref().map(TorOutboundProxy::uri).transpose()?;
	if matches!(proxy, Some(TorOutboundProxy::Http(_))) && url.scheme_str() == Some("http") {
		*request.uri_mut() = url.clone();
	}
	let isolation = state.isolation.take().unwrap_or_else(|| {
		let mut prefs = StreamPrefs::default();
		prefs.new_isolation_group();
		(prefs, Some(format!("{:032x}", rand::random::<u128>())))
	});
	let mut cached = state.cached.take().filter(|(target, connection)| {
		target.scheme() == url.scheme()
			&& target.authority() == url.authority()
			&& !connection.sender.is_closed()
	});
	loop {
		if Instant::now() >= deadline {
			return Err(Error::Post(PostError::NotSent(
				"Tor request timed out".into(),
			)));
		}
		let reused = cached.is_some();
		let mut connection = match cached.take() {
			Some((_, connection)) => connection,
			None => {
				let connect = async {
					if let Some(proxy) = &proxy {
						match proxy {
							TorOutboundProxy::Socks5(address) => {
								connect_socks(address.clone(), &url, isolation.1.as_deref()).await
							}
							TorOutboundProxy::Http(_) => {
								connect_http(proxy_uri.clone().unwrap(), &url).await
							}
						}
					} else if config.use_integrated() {
						crate::tor::arti::connect(config, &url, &isolation.0)
							.await
							.map_err(Error::Post)
					} else {
						connect_socks(config.socks_proxy_addr(), &url, isolation.1.as_deref()).await
					}
				};
				tokio::select! {
					biased;
					_ = closed.changed() => return Err(Error::Post(PostError::NotSent("Tor context closed".into()))),
					result = timeout_at(deadline, connect) => result.map_err(|_| Error::Post(PostError::NotSent("Tor request timed out".into())))??,
				}
			}
		};
		let response = connection
			.send(request.clone(), limit, deadline, closed)
			.await;
		if reused && matches!(response, Err(PostError::NotSent(_))) {
			continue;
		}
		if response.is_ok() || matches!(response, Err(PostError::PossiblySent(_, Some(_), Some(_))))
		{
			state.isolation = Some(isolation);
			if !connection.sender.is_closed() {
				state.cached = Some((url, connection));
			}
		}
		return response.map_err(Error::Post);
	}
}

async fn connect_socks(
	mut addr: String,
	url: &Uri,
	isolation: Option<&str>,
) -> Result<HttpConnection, Error> {
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
	let mut proxy = hyper_socks2::SocksConnector {
		proxy_addr: uri,
		auth: isolation.map(|token| hyper_socks2::Auth::new(token, "tor")),
		connector,
	}
	.with_tls()
	.map_err(|err| Error::Generic(format!("Unable to create socks connector: {}", err)))?;
	let stream = proxy.call(url.clone()).await.map_err(|e| {
		Error::Post(PostError::NotSent(format!(
			"Unable to connect over socks proxy: {:?}",
			e
		)))
	})?;
	HttpConnection::open(stream).await.map_err(Error::Post)
}

async fn connect_http(proxy: Uri, url: &Uri) -> Result<HttpConnection, Error> {
	let proxy = hyper_proxy2::Proxy::new(hyper_proxy2::Intercept::All, proxy);
	let mut connector = hyper_proxy2::ProxyConnector::from_proxy(HttpsConnector::new(), proxy)
		.map_err(|e| Error::Config(format!("Unable to create HTTP proxy connector: {}", e)))?;
	let stream = connector
		.call(url.clone())
		.await
		.map_err(|e| Error::Post(PostError::NotSent(format!("HTTP proxy connection: {}", e))))?;
	HttpConnection::open(stream).await.map_err(Error::Post)
}

pub(super) struct HttpConnection {
	pub sender: hyper::client::conn::http1::SendRequest<Full<Bytes>>,
	pub driver: BoxFuture<'static, Result<(), hyper::Error>>,
}

impl HttpConnection {
	pub async fn open<I>(io: I) -> Result<Self, PostError>
	where
		I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
	{
		let (sender, driver) = hyper::client::conn::http1::handshake(io)
			.await
			.map_err(|e| PostError::NotSent(format!("Handshake: {:?}", e)))?;
		Ok(Self {
			sender,
			driver: Box::pin(driver.fuse()),
		})
	}

	async fn send(
		&mut self,
		request: Request<Full<Bytes>>,
		limit: usize,
		deadline: Instant,
		closed: &mut watch::Receiver<bool>,
	) -> Result<Response<String>, PostError> {
		let mut status = None;
		let result = {
			let response = async {
				let response = self.sender.try_send_request(request).await.map_err(|e| {
					let message = format!("{:?}", e.error());
					if e.message().is_some() {
						PostError::NotSent(message)
					} else {
						PostError::PossiblySent(message, None, None)
					}
				})?;
				status = Some(response.status());
				read_response(response, limit).await
			};
			let transfer = async {
				tokio::pin!(response);
				tokio::select! {
					result = &mut response => result,
					_ = &mut self.driver => response.await,
				}
			};
			tokio::select! {
				biased;
				_ = closed.changed() => Err("Tor context closed"),
				result = timeout_at(deadline, transfer) => result.map_err(|_| "Tor request timed out"),
			}
		};
		result.map_err(|e| PostError::PossiblySent(e.into(), status, None))?
	}
}

/// Build an origin-form request
fn build_request(
	method: Method,
	body: String,
	headers: Option<HashMap<&str, &str>>,
	url: &str,
) -> Result<(Uri, Request<Full<Bytes>>), PostError> {
	let url: Uri = url
		.parse()
		.map_err(|_| PostError::NotSent(format!("Bad URL: {}", url)))?;
	if !matches!(url.scheme_str(), Some("http" | "https")) || url.host().is_none() {
		return Err(PostError::NotSent(format!("Bad URL: {}", url)));
	}
	let mut req = Request::builder()
		.method(method)
		.uri(url.path_and_query().map(|p| p.as_str()).unwrap_or("/"));
	if let Some(headers) = headers {
		for (name, value) in headers {
			req = req.header(name, value);
		}
	}
	if !req.headers_ref().is_some_and(|h| h.contains_key("host")) {
		req = req.header("host", url.authority().unwrap().as_str());
	}
	let request = req
		.body(Full::from(body))
		.map_err(|e| PostError::NotSent(format!("Cannot build request: {}", e)))?;
	Ok((url, request))
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
	use hyper_util::rt::TokioIo;
	use std::io;
	use std::time::Duration;
	use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
	use tokio::sync::oneshot;
	use tokio::time::timeout;

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

	async fn send_request(
		stream: DuplexStream,
		request: Request<Full<Bytes>>,
		limit: usize,
		deadline: Instant,
	) -> Result<Response<String>, PostError> {
		let (_close, mut closed) = watch::channel(false);
		HttpConnection::open(TokioIo::new(stream))
			.await?
			.send(request, limit, deadline, &mut closed)
			.await
	}
	#[test]
	fn builds_origin_form_request() {
		let url = "http://example.onion:8080/v1?test=1";
		let mut headers = HashMap::new();
		headers.insert("key", "value");
		let (_, request) =
			build_request(Method::POST, "{}".to_string(), Some(headers), url).unwrap();

		assert_eq!(request.method(), Method::POST);
		assert_eq!(request.uri(), "/v1?test=1");
		assert_eq!(request.headers()["host"], "example.onion:8080");
		assert_eq!(request.headers()["key"], "value");
		let mut headers = HashMap::new();
		headers.insert("Host", "custom.onion");
		let (_, request) = build_request(Method::POST, String::new(), Some(headers), url).unwrap();
		assert_eq!(request.headers().get_all("host").iter().count(), 1);
		assert_eq!(request.headers()["host"], "custom.onion");
	}

	async fn read_headers(stream: &mut DuplexStream) {
		let mut headers = Vec::new();
		while !headers.ends_with(b"\r\n\r\n") {
			headers.push(stream.read_u8().await.unwrap());
		}
	}

	#[tokio::test]
	async fn connection_reuse() {
		let (stream, mut peer) = tokio::io::duplex(1024);
		let server = tokio::spawn(async move {
			for _ in 0..24 {
				read_headers(&mut peer).await;
				peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
					.await
					.unwrap();
			}
			assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
		});
		let mut connection = HttpConnection::open(TokioIo::new(stream)).await.unwrap();
		let (_close, mut closed) = watch::channel(false);
		for _ in 0..24 {
			let (_, request) =
				build_request(Method::POST, String::new(), None, "http://example.onion/").unwrap();
			let response = connection
				.send(
					request,
					2,
					Instant::now() + Duration::from_secs(1),
					&mut closed,
				)
				.await
				.unwrap();
			assert_eq!(response.body(), "ok");
		}
		drop(connection);
		timeout(Duration::from_secs(1), server)
			.await
			.unwrap()
			.unwrap();
	}

	#[tokio::test]
	async fn idle_close() {
		let (stream, mut peer) = tokio::io::duplex(1024);
		let (close, idle) = oneshot::channel();
		let server = tokio::spawn(async move {
			read_headers(&mut peer).await;
			peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
				.await
				.unwrap();
			idle.await.unwrap();
		});
		let mut connection = HttpConnection::open(TokioIo::new(stream)).await.unwrap();
		let (_close, mut closed) = watch::channel(false);
		let (_, request) =
			build_request(Method::POST, String::new(), None, "http://example.onion/").unwrap();
		let deadline = Instant::now() + Duration::from_secs(1);
		connection
			.send(request.clone(), 0, deadline, &mut closed)
			.await
			.unwrap();
		close.send(()).unwrap();
		server.await.unwrap();
		assert!(!connection.sender.is_closed());
		let result = connection.send(request, 0, deadline, &mut closed).await;
		assert!(matches!(result, Err(PostError::NotSent(_))), "{:?}", result);
	}

	#[tokio::test]
	async fn context_close() {
		let context = std::sync::Arc::new(TorContext::new(()));
		let (stream, mut peer) = tokio::io::duplex(1024);
		let (url, request) =
			build_request(Method::POST, String::new(), None, "http://example.onion/").unwrap();
		context.state.lock().await.cached = Some((
			url,
			HttpConnection::open(TokioIo::new(stream)).await.unwrap(),
		));
		let (sent, received) = oneshot::channel();
		let server = tokio::spawn(async move {
			read_headers(&mut peer).await;
			peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nx")
				.await
				.unwrap();
			sent.send(()).unwrap();
			assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
		});
		let active = context.clone();
		let request = tokio::spawn(async move {
			let mut closed = active.closed.subscribe();
			let mut state = active.state.lock().await;
			let (_, mut connection) = state.cached.take().unwrap();
			connection
				.send(
					request,
					2,
					Instant::now() + Duration::from_secs(5),
					&mut closed,
				)
				.await
		});
		timeout(Duration::from_secs(1), received)
			.await
			.unwrap()
			.unwrap();
		timeout(Duration::from_secs(1), context.close())
			.await
			.unwrap();
		assert!(matches!(
			request.await.unwrap(),
			Err(PostError::PossiblySent(..))
		));
		timeout(Duration::from_secs(1), server)
			.await
			.unwrap()
			.unwrap();
		assert!(context.state.lock().await.cached.is_none());
		context.close().await;
	}

	#[tokio::test]
	async fn head_response() {
		let (stream, mut peer) = tokio::io::duplex(1024);
		let server = tokio::spawn(async move {
			read_headers(&mut peer).await;
			peer.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 100\r\n\r\n")
				.await
				.unwrap();
			assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
		});
		let url = "http://example.onion/";
		let (_, request) = build_request(Method::HEAD, String::new(), None, url).unwrap();
		let result =
			send_request(stream, request, 0, Instant::now() + Duration::from_secs(1)).await;
		assert!(
			matches!(result, Err(super::PostError::PossiblySent(_, Some(status), Some(ref body))) if status.as_u16() == 503 && body.is_empty())
		);
		server.await.unwrap();
	}

	#[tokio::test]
	async fn request_cancel() {
		for response in ["", "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nx"] {
			let (stream, mut peer) = tokio::io::duplex(1024);
			let (sent, received) = oneshot::channel();
			let server = tokio::spawn(async move {
				read_headers(&mut peer).await;
				peer.write_all(response.as_bytes()).await.unwrap();
				sent.send(()).unwrap();
				assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
			});
			let url = "http://example.onion/";
			let (_, request) = build_request(Method::POST, String::new(), None, url).unwrap();
			let task = tokio::spawn(async move {
				send_request(
					stream,
					request,
					100,
					Instant::now() + Duration::from_secs(1),
				)
				.await
			});
			timeout(Duration::from_secs(1), received)
				.await
				.unwrap()
				.unwrap();
			task.abort();
			assert!(task.await.unwrap_err().is_cancelled());
			timeout(Duration::from_secs(1), server)
				.await
				.unwrap()
				.unwrap();
		}
	}

	#[tokio::test]
	async fn request_complete() {
		let (stream, mut peer) = tokio::io::duplex(1024);
		let server = async move {
			read_headers(&mut peer).await;
			peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
				.await
				.unwrap();
			assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
		};
		let url = "http://example.onion/";
		let (_, request) = build_request(Method::POST, String::new(), None, url).unwrap();
		let deadline = Instant::now() + Duration::from_secs(1);
		let response = send_request(stream, request, 2, deadline);
		let (_, response) = timeout(Duration::from_secs(1), async {
			tokio::join!(server, response)
		})
		.await
		.unwrap();
		assert_eq!(response.unwrap().body(), "ok");
	}
}
