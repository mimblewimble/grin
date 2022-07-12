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

//! RESTful API server to easily expose services as RESTful JSON/HTTP endpoints.
//! Fairly constrained on what the service API must look like by design.
//!
//! To use it, just have your service(s) implement the ApiEndpoint trait and
//! register them on a ApiServer.

use crate::router::{Handler, HandlerObj, ResponseFuture, Router, RouterError};
use crate::web::response;
use futures::channel::oneshot;
use futures::TryStreamExt;
use hyper::server::accept;
use hyper::service::make_service_fn;
use hyper::{Body, Request, Server, StatusCode};
use rustls::internal::pemfile;
use std::convert::Infallible;
use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{io, thread};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::stream::StreamExt;
use tokio_rustls::TlsAcceptor;

/// Errors that can be returned by an ApiEndpoint implementation.
#[derive(Clone, Eq, PartialEq, Debug, thiserror::Error, Serialize, Deserialize)]
pub enum Error {
	#[error("Internal error: {0}")]
	Internal(String),
	#[error("Bad arguments: {0}")]
	Argument(String),
	#[error("Not found.")]
	NotFound,
	#[error("Request error: {0}")]
	RequestError(String),
	#[error("ResponseError error: {0}")]
	ResponseError(String),
	#[error("Router error: {source}")]
	Router {
		#[from]
		source: RouterError,
	},
}

impl From<crate::chain::Error> for Error {
	fn from(error: crate::chain::Error) -> Error {
		Error::Internal(error.to_string())
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
		let certfile = File::open(&self.certificate).map_err(|e| {
			Error::Internal(format!("failed to open file {} {}", self.certificate, e))
		})?;
		let mut reader = io::BufReader::new(certfile);

		pemfile::certs(&mut reader)
			.map_err(|_| Error::Internal("failed to load certificate".to_string()))
	}

	fn load_private_key(&self) -> Result<rustls::PrivateKey, Error> {
		let keyfile = File::open(&self.private_key)
			.map_err(|e| Error::Internal(format!("failed to open private key file {}", e)))?;
		let mut reader = io::BufReader::new(keyfile);

		let keys = pemfile::pkcs8_private_keys(&mut reader)
			.map_err(|_| Error::Internal("failed to load private key".to_string()))?;
		if keys.len() != 1 {
			return Err(Error::Internal("expected a single private key".to_string()));
		}
		Ok(keys[0].clone())
	}

	pub fn build_server_config(&self) -> Result<Arc<rustls::ServerConfig>, Error> {
		let certs = self.load_certs()?;
		let key = self.load_private_key()?;
		let mut cfg = rustls::ServerConfig::new(rustls::NoClientAuth::new());
		cfg.set_single_cert(certs, key)
			.map_err(|e| Error::Internal(format!("set single certificate failed {}", e)))?;
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
		api_chan: &'static mut (oneshot::Sender<()>, oneshot::Receiver<()>),
	) -> Result<thread::JoinHandle<()>, Error> {
		match conf {
			Some(conf) => self.start_tls(addr, router, conf, api_chan),
			None => self.start_no_tls(addr, router, api_chan),
		}
	}

	/// Starts the ApiServer at the provided address.
	fn start_no_tls(
		&mut self,
		addr: SocketAddr,
		router: Router,
		api_chan: &'static mut (oneshot::Sender<()>, oneshot::Receiver<()>),
	) -> Result<thread::JoinHandle<()>, Error> {
		if self.shutdown_sender.is_some() {
			return Err(Error::Internal(
				"Can't start HTTP API server, it's running already".to_string(),
			));
		}
		let rx = &mut api_chan.1;
		let tx = &mut api_chan.0;

		// Jones's trick to update memory
		let m = oneshot::channel::<()>();
		let tx = std::mem::replace(tx, m.0);
		self.shutdown_sender = Some(tx);

		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let server = async move {
					let server = Server::bind(&addr)
						.serve(make_service_fn(move |_| {
							let router = router.clone();
							async move { Ok::<_, Infallible>(router) }
						}))
						.with_graceful_shutdown(async {
							rx.await.ok();
						});

					server.await
				};

				let mut rt = Runtime::new()
					.map_err(|e| eprintln!("HTTP API server error: {}", e))
					.unwrap();
				if let Err(e) = rt.block_on(server) {
					eprintln!("HTTP API server error: {}", e)
				}
			})
			.map_err(|_| Error::Internal("failed to spawn API thread".to_string()))
	}

	/// Starts the TLS ApiServer at the provided address.
	/// TODO support stop operation
	fn start_tls(
		&mut self,
		addr: SocketAddr,
		router: Router,
		conf: TLSConfig,
		api_chan: &'static mut (oneshot::Sender<()>, oneshot::Receiver<()>),
	) -> Result<thread::JoinHandle<()>, Error> {
		if self.shutdown_sender.is_some() {
			return Err(Error::Internal(
				"Can't start HTTPS API server, it's running already".to_string(),
			));
		}

		let rx = &mut api_chan.1;
		let tx = &mut api_chan.0;

		// Jones's trick to update memory
		let m = oneshot::channel::<()>();
		let tx = std::mem::replace(tx, m.0);
		self.shutdown_sender = Some(tx);

		let acceptor = TlsAcceptor::from(conf.build_server_config()?);

		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let server = async move {
					let mut listener = TcpListener::bind(&addr).await.expect("failed to bind");
					let listener = listener
						.incoming()
						.and_then(move |s| acceptor.accept(s))
						.filter(|r| r.is_ok());

					let server = Server::builder(accept::from_stream(listener))
						.serve(make_service_fn(move |_| {
							let router = router.clone();
							async move { Ok::<_, Infallible>(router) }
						}))
						.with_graceful_shutdown(async {
							rx.await.ok();
						});

					server.await
				};

				let mut rt = Runtime::new()
					.map_err(|e| eprintln!("HTTP API server error: {}", e))
					.unwrap();
				if let Err(e) = rt.block_on(server) {
					eprintln!("HTTP API server error: {}", e)
				}
			})
			.map_err(|_| Error::Internal("failed to spawn API thread".to_string()))
	}

	/// Stops the API server, it panics in case of error
	pub fn stop(&mut self) -> bool {
		if self.shutdown_sender.is_some() {
			let tx = self.shutdown_sender.as_mut().unwrap();
			let m = oneshot::channel::<()>();
			let tx = std::mem::replace(tx, m.0);
			tx.send(()).expect("Failed to stop API server");
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
