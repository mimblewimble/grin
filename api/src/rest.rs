// Copyright 2020 The Grin Developers
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

//! RESTful API server to easily expose services as RESTful JSON/HTTP endpoints.
//! Fairly constrained on what the service API must look like by design.
//!
//! To use it, just have your service(s) implement the ApiEndpoint trait and
//! register them on a ApiServer.

use crate::router::{Handler, HandlerObj, ResponseFuture, Router, RouterError};
use crate::web::response;
use failure::{Backtrace, Context, Fail, ResultExt};
use futures::channel::oneshot;
use futures::TryStreamExt;
use hyper::server::accept;
use hyper::service::make_service_fn;
use hyper::{Body, Request, Server, StatusCode};
use rustls;
use rustls::internal::pemfile;
use std::convert::Infallible;
use std::fmt::{self, Display};
use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{io, thread};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio_rustls::TlsAcceptor;

/// Errors that can be returned by an ApiEndpoint implementation.
#[derive(Debug)]
pub struct Error {
	inner: Context<ErrorKind>,
}

#[derive(Clone, Eq, PartialEq, Debug, Fail, Serialize, Deserialize)]
pub enum ErrorKind {
	#[fail(display = "Internal error: {}", _0)]
	Internal(String),
	#[fail(display = "Bad arguments: {}", _0)]
	Argument(String),
	#[fail(display = "Not found.")]
	NotFound,
	#[fail(display = "Request error: {}", _0)]
	RequestError(String),
	#[fail(display = "ResponseError error: {}", _0)]
	ResponseError(String),
	#[fail(display = "Router error: {}", _0)]
	Router(RouterError),
}

impl Fail for Error {
	fn cause(&self) -> Option<&dyn Fail> {
		self.inner.cause()
	}

	fn backtrace(&self) -> Option<&Backtrace> {
		self.inner.backtrace()
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		Display::fmt(&self.inner, f)
	}
}

impl Error {
	pub fn kind(&self) -> &ErrorKind {
		self.inner.get_context()
	}
}

impl From<ErrorKind> for Error {
	fn from(kind: ErrorKind) -> Error {
		Error {
			inner: Context::new(kind),
		}
	}
}

impl From<Context<ErrorKind>> for Error {
	fn from(inner: Context<ErrorKind>) -> Error {
		Error { inner: inner }
	}
}

impl From<RouterError> for Error {
	fn from(error: RouterError) -> Error {
		Error {
			inner: Context::new(ErrorKind::Router(error)),
		}
	}
}

impl From<crate::chain::Error> for Error {
	fn from(error: crate::chain::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Internal(error.to_string())),
		}
	}
}

/// TLS config
#[derive(Clone)]
pub struct TLSConfig {
	pub certificate: String,
	pub private_key: String,
}

impl TLSConfig {
	pub fn new(certificate: String, private_key: String) -> TLSConfig {
		TLSConfig {
			certificate,
			private_key,
		}
	}

	fn load_certs(&self) -> Result<Vec<rustls::Certificate>, Error> {
		let certfile = File::open(&self.certificate).context(ErrorKind::Internal(format!(
			"failed to open file {}",
			self.certificate
		)))?;
		let mut reader = io::BufReader::new(certfile);

		pemfile::certs(&mut reader)
			.map_err(|_| ErrorKind::Internal("failed to load certificate".to_string()).into())
	}

	fn load_private_key(&self) -> Result<rustls::PrivateKey, Error> {
		let keyfile = File::open(&self.private_key).context(ErrorKind::Internal(format!(
			"failed to open file {}",
			self.private_key
		)))?;
		let mut reader = io::BufReader::new(keyfile);

		let keys = pemfile::pkcs8_private_keys(&mut reader)
			.map_err(|_| ErrorKind::Internal("failed to load private key".to_string()))?;
		if keys.len() != 1 {
			return Err(ErrorKind::Internal("expected a single private key".to_string()).into());
		}
		Ok(keys[0].clone())
	}

	pub fn build_server_config(&self) -> Result<Arc<rustls::ServerConfig>, Error> {
		let certs = self.load_certs()?;
		let key = self.load_private_key()?;
		let mut cfg = rustls::ServerConfig::new(rustls::NoClientAuth::new());
		cfg.set_single_cert(certs, key)
			.context(ErrorKind::Internal(
				"set single certificate failed".to_string(),
			))?;
		Ok(Arc::new(cfg))
	}
}

/// HTTP server allowing the registration of ApiEndpoint implementations.
pub struct ApiServer {
	shutdown_sender: Option<oneshot::Sender<()>>,
}

impl ApiServer {
	/// Creates a new ApiServer that will serve ApiEndpoint implementations
	/// under the root URL.
	pub fn new() -> ApiServer {
		ApiServer {
			shutdown_sender: None,
		}
	}

	/// Starts ApiServer at the provided address.
	/// TODO support stop operation
	pub fn start(
		&mut self,
		addr: SocketAddr,
		router: Router,
		conf: Option<TLSConfig>,
	) -> Result<thread::JoinHandle<()>, Error> {
		match conf {
			Some(conf) => self.start_tls(addr, router, conf),
			None => self.start_no_tls(addr, router),
		}
	}

	/// Starts the ApiServer at the provided address.
	fn start_no_tls(
		&mut self,
		addr: SocketAddr,
		router: Router,
	) -> Result<thread::JoinHandle<()>, Error> {
		if self.shutdown_sender.is_some() {
			return Err(ErrorKind::Internal(
				"Can't start HTTP API server, it's running already".to_string(),
			)
			.into());
		}
		let (tx, _rx) = oneshot::channel::<()>();
		self.shutdown_sender = Some(tx);
		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let server = async move {
					let server = Server::bind(&addr).serve(make_service_fn(move |_| {
						let router = router.clone();
						async move { Ok::<_, Infallible>(router) }
					}));
					// TODO graceful shutdown is unstable, investigate
					//.with_graceful_shutdown(rx)

					server.await
				};

				let mut rt = Runtime::new()
					.map_err(|e| eprintln!("HTTP API server error: {}", e))
					.unwrap();
				if let Err(e) = rt.block_on(server) {
					eprintln!("HTTP API server error: {}", e)
				}
			})
			.map_err(|_| ErrorKind::Internal("failed to spawn API thread".to_string()).into())
	}

	/// Starts the TLS ApiServer at the provided address.
	/// TODO support stop operation
	fn start_tls(
		&mut self,
		addr: SocketAddr,
		router: Router,
		conf: TLSConfig,
	) -> Result<thread::JoinHandle<()>, Error> {
		if self.shutdown_sender.is_some() {
			return Err(ErrorKind::Internal(
				"Can't start HTTPS API server, it's running already".to_string(),
			)
			.into());
		}

		let acceptor = TlsAcceptor::from(conf.build_server_config()?);

		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let server = async move {
					let mut listener = TcpListener::bind(&addr).await.expect("failed to bind");
					let listener = listener.incoming().and_then(move |s| acceptor.accept(s));

					let server = Server::builder(accept::from_stream(listener)).serve(
						make_service_fn(move |_| {
							let router = router.clone();
							async move { Ok::<_, Infallible>(router) }
						}),
					);

					server.await
				};

				let mut rt = Runtime::new()
					.map_err(|e| eprintln!("HTTP API server error: {}", e))
					.unwrap();
				if let Err(e) = rt.block_on(server) {
					eprintln!("HTTP API server error: {}", e)
				}
			})
			.map_err(|_| ErrorKind::Internal("failed to spawn API thread".to_string()).into())
	}

	/// Stops the API server, it panics in case of error
	pub fn stop(&mut self) -> bool {
		if self.shutdown_sender.is_some() {
			// TODO re-enable stop after investigation
			//let tx = mem::replace(&mut self.shutdown_sender, None).unwrap();
			//tx.send(()).expect("Failed to stop API server");
			info!("API server has been stopped");
			true
		} else {
			error!("Can't stop API server, it's not running or doesn't spport stop operation");
			false
		}
	}
}

pub struct LoggingMiddleware {}

impl Handler for LoggingMiddleware {
	fn call(
		&self,
		req: Request<Body>,
		mut handlers: Box<dyn Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		debug!("REST call: {} {}", req.method(), req.uri().path());
		match handlers.next() {
			Some(handler) => handler.call(req, handlers),
			None => response(StatusCode::INTERNAL_SERVER_ERROR, "no handler found"),
		}
	}
}
