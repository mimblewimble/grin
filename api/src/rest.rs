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

//! RESTful API server to easily expose services as RESTful JSON/HTTP endpoints.
//! Fairly constrained on what the service API must look like by design.
//!
//! To use it, just have your service(s) implement the ApiEndpoint trait and
//! register them on a ApiServer.

use failure::{Backtrace, Context, Fail};
use futures::sync::oneshot;
use futures::Stream;
use hyper::rt::Future;
use hyper::server::conn::Http;
use hyper::{rt, Body, Request, Server};
use native_tls::{Identity, TlsAcceptor};
use router::{Handler, HandlerObj, ResponseFuture, Router};
use std::fmt::{self, Display};
use std::net::SocketAddr;
use std::{io, thread};
use tokio::net::TcpListener;
use tokio_tls;
use util::LOGGER;

/// Errors that can be returned by an ApiEndpoint implementation.
#[derive(Debug)]
pub struct Error {
	inner: Context<ErrorKind>,
}

#[derive(Clone, Eq, PartialEq, Debug, Fail)]
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
}

impl Fail for Error {
	fn cause(&self) -> Option<&Fail> {
		self.inner.cause()
	}

	fn backtrace(&self) -> Option<&Backtrace> {
		self.inner.backtrace()
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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

/// TLS config
pub struct TLSConfig {
	pub pkcs_bytes: Vec<u8>,
	pub pass: String,
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

	/// Starts the ApiServer at the provided address.
	pub fn start(
		&mut self,
		addr: SocketAddr,
		router: Router,
	) -> Result<thread::JoinHandle<()>, Error> {
		if self.shutdown_sender.is_some() {
			return Err(ErrorKind::Internal(
				"Can't start HTTP API server, it's running already".to_string(),
			))?;
		}
		let (tx, _rx) = oneshot::channel::<()>();
		self.shutdown_sender = Some(tx);
		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let server = Server::bind(&addr)
					.serve(router)
					// TODO graceful shutdown is unstable, investigate 
					//.with_graceful_shutdown(rx)
					.map_err(|e| eprintln!("HTTP API server error: {}", e));

				rt::run(server);
			})
			.map_err(|_| ErrorKind::Internal("failed to spawn API thread".to_string()).into())
	}

	/// Starts the TLS ApiServer at the provided address.
	/// TODO support stop operation
	pub fn start_tls(
		&mut self,
		addr: SocketAddr,
		router: Router,
		conf: TLSConfig,
	) -> Result<thread::JoinHandle<()>, Error> {
		if self.shutdown_sender.is_some() {
			return Err(ErrorKind::Internal(
				"Can't start HTTPS API server, it's running already".to_string(),
			))?;
		}
		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let cert = Identity::from_pkcs12(conf.pkcs_bytes.as_slice(), &conf.pass).unwrap();
				let tls_cx = TlsAcceptor::builder(cert).build().unwrap();
				let tls_cx = tokio_tls::TlsAcceptor::from(tls_cx);
				let srv = TcpListener::bind(&addr).expect("Error binding local port");
				// Use lower lever hyper API to be able to intercept client connection
				let server = Http::new()
					.serve_incoming(
						srv.incoming().and_then(move |socket| {
							tls_cx
								.accept(socket)
								.map_err(|e| io::Error::new(io::ErrorKind::Other, e))
						}),
						router,
					)
					.then(|res| match res {
						Ok(conn) => Ok(Some(conn)),
						Err(e) => {
							eprintln!("Error: {}", e);
							Ok(None)
						}
					})
					.for_each(|conn_opt| {
						if let Some(conn) = conn_opt {
							rt::spawn(
								conn.and_then(|c| c.map_err(|e| panic!("Hyper error {}", e)))
									.map_err(|e| eprintln!("Connection error {}", e)),
							);
						}
						Ok(())
					});

				rt::run(server);
			})
			.map_err(|_| ErrorKind::Internal("failed to spawn API thread".to_string()).into())
	}

	/// Stops the API server, it panics in case of error
	pub fn stop(&mut self) -> bool {
		if self.shutdown_sender.is_some() {
			// TODO re-enable stop after investigation
			//let tx = mem::replace(&mut self.shutdown_sender, None).unwrap();
			//tx.send(()).expect("Failed to stop API server");
			info!(LOGGER, "API server has been stoped");
			true
		} else {
			error!(
				LOGGER,
				"Can't stop API server, it's not running or doesn't spport stop operation"
			);
			false
		}
	}
}

pub struct LoggingMiddleware {}

impl Handler for LoggingMiddleware {
	fn call(
		&self,
		req: Request<Body>,
		mut handlers: Box<Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		debug!(LOGGER, "REST call: {} {}", req.method(), req.uri().path());
		handlers.next().unwrap().call(req, handlers)
	}
}
