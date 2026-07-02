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
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::pem::{PemObject, SectionKind};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile as pemfile;
use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{io, thread};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
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

	pub fn build_server_config(&self) -> Result<Arc<rustls::ServerConfig>, Error> {
		let certs: Vec<CertificateDer> = {
			let certfile = File::open(&self.certificate).map_err(|e| {
				Error::Internal(format!("failed to open file {} {}", self.certificate, e))
			})?;
			let mut reader = io::BufReader::new(certfile);

			let certs = pemfile::certs(&mut reader)
				.map_err(|_| Error::Internal("failed to load certificate".to_string()))?;
			certs
				.into_iter()
				.map(CertificateDer::from)
				.collect::<Vec<CertificateDer>>()
		};
		let key = {
			let keyfile = File::open(&self.private_key)
				.map_err(|e| Error::Internal(format!("failed to open private key file {}", e)))?;
			let mut reader = io::BufReader::new(keyfile);

			let keys = pemfile::pkcs8_private_keys(&mut reader)
				.map_err(|_| Error::Internal("failed to load private key".to_string()))?;
			if keys.len() != 1 {
				return Err(Error::Internal("expected a single private key".to_string()));
			}
			let key = PrivateKeyDer::from_pem(SectionKind::PrivateKey, keys[0].clone());
			match key {
				None => return Err(Error::Internal("wrong private key type".to_string())),
				Some(k) => k,
			}
		};

		let cfg = rustls::ServerConfig::builder()
			.with_no_client_auth()
			.with_single_cert(certs, key)
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
					let graceful = hyper_util::server::graceful::GracefulShutdown::new();
					// When this signal completes, start shutdown.
					let mut signal = std::pin::pin!(shutdown_signal(rx));

					// Start server loop.
					match TcpListener::bind(addr).await {
						Ok(l) => loop {
							tokio::select! {
								Ok(s) = async {
									match l.accept().await {
										Ok((s, _)) => Ok(s),
										Err(e) => {
											error!("Failed to accept connection: {e:#}");
											Err(e)
										}
									}
								} => {
									let io = TokioIo::new(s);
									let router = router.clone();
									let conn = http1::Builder::new().serve_connection(io, router);
									let fut = graceful.watch(conn);
									tokio::spawn(async move {
										if let Err(e) = fut.await {
											error!("API server error: {:?}", e);
										}
									});
								}
								_ = &mut signal => {
									drop(l);
									break;
								}
							}
						},
						Err(e) => {
							error!("API listener binding error: {}", e);
						}
					}
				};

				let rt = Runtime::new()
					.map_err(|e| error!("HTTP API server error: {}", e))
					.unwrap();
				rt.block_on(server);
			})
			.map_err(|_| Error::Internal("failed to spawn API thread".to_string()))
	}

	/// Starts the TLS ApiServer at the provided address.
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

		let tls_acceptor = TlsAcceptor::from(conf.build_server_config()?);

		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || {
				let server = async move {
					let graceful = hyper_util::server::graceful::GracefulShutdown::new();
					// When this signal completes, start shutdown.
					let mut signal = std::pin::pin!(shutdown_signal(rx));

					// Start server loop.
					match TcpListener::bind(addr).await {
						Ok(l) => loop {
							tokio::select! {
								Ok(s) = async {
									match l.accept().await {
										Ok((s, _)) => Ok(s),
										Err(e) => {
											error!("Failed to accept connection: {e:#}");
											Err(e)
										}
									}
								} => {
									let router = router.clone();
									let tls_acceptor = tls_acceptor.clone();
									let tls_stream = match tls_acceptor.accept(s).await {
										Ok(tls_stream) => tls_stream,
										Err(err) => {
											error!("failed to perform TLS handshake: {err:#}");
											continue;
										}
									};
									let io = TokioIo::new(tls_stream);
									let router = router.clone();
									let conn = http1::Builder::new().serve_connection(io, router);
									let fut = graceful.watch(conn);
									tokio::spawn(async move {
										if let Err(e) = fut.await {
											error!("API server error: {:?}", e);
										}
									});
								}
								_ = &mut signal => {
									drop(l);
									break;
								}
							}
						},
						Err(e) => {
							error!("API listener binding error: {}", e);
						}
					}
				};

				let rt = Runtime::new()
					.map_err(|e| eprintln!("HTTP API server error: {}", e))
					.unwrap();
				rt.block_on(server);
			})
			.map_err(|_| Error::Internal("failed to spawn API thread".to_string()))
	}

	/// Stops the API server.
	pub fn stop(&mut self) -> bool {
		if self.shutdown_sender.is_some() {
			let tx = self.shutdown_sender.as_mut().unwrap();
			let m = oneshot::channel::<()>();
			let tx = std::mem::replace(tx, m.0);
			match tx.send(()) {
				Ok(_) => {
					info!("API server has been stopped");
					true
				}
				Err(_) => {
					error!("Failed to stop API server");
					false
				}
			}
		} else {
			error!("Can't stop API server, it's not running or doesn't spport stop operation");
			false
		}
	}
}

/// Signal for graceful API server shutdown.
async fn shutdown_signal(rx: &mut oneshot::Receiver<()>) {
	rx.await.ok();
}

pub struct LoggingMiddleware {}

impl Handler for LoggingMiddleware {
	fn call(
		&self,
		req: Request<Incoming>,
		mut handlers: Box<dyn Iterator<Item = HandlerObj>>,
	) -> ResponseFuture {
		debug!("REST call: {} {}", req.method(), req.uri().path());
		match handlers.next() {
			Some(handler) => handler.call(req, handlers),
			None => response(StatusCode::INTERNAL_SERVER_ERROR, "no handler found".into()),
		}
	}
}
