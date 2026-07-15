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
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Duration};
use tokio_rustls::TlsAcceptor;

const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

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
	shutdown_sender: Option<mpsc::Sender<()>>,
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
	pub fn start(
		&mut self,
		addr: SocketAddr,
		router: Router,
		conf: Option<TLSConfig>,
		api_chan: (mpsc::Sender<()>, mpsc::Receiver<()>),
	) -> Result<thread::JoinHandle<()>, Error> {
		let _ = rustls::crypto::ring::default_provider().install_default();

		if self.shutdown_sender.is_some() {
			return Err(Error::Internal(
				"Can't start API server, it's running already".to_string(),
			));
		}
		self.shutdown_sender = Some(api_chan.0);

		let tls = match conf {
			Some(conf) => Some(TlsAcceptor::from(conf.build_server_config()?)),
			None => None,
		};
		thread::Builder::new()
			.name("apis".to_string())
			.spawn(move || start_server(addr, router, api_chan.1, tls))
			.map_err(|_| Error::Internal("failed to spawn API thread".to_string()))
	}

	/// Stops the API server.
	pub fn stop(&mut self) -> bool {
		if let Some(tx) = self.shutdown_sender.take() {
			match tx.try_send(()) {
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
			error!("Can't stop API server, it's not running or already stopped");
			false
		}
	}
}

/// Start API server with optional TLS support.
fn start_server(
	addr: SocketAddr,
	router: Router,
	rx: mpsc::Receiver<()>,
	tls: Option<TlsAcceptor>,
) {
	let server = async move {
		let graceful = hyper_util::server::graceful::GracefulShutdown::new();
		// When this signal completes, start shutdown.
		let mut signal = std::pin::pin!(shutdown_signal(rx));

		// Start server loop.
		match TcpListener::bind(addr).await {
			Ok(l) => {
				loop {
					tokio::select! {
						Ok(s) = async {
							match l.accept().await {
								Ok((s, _)) => Ok::<Option<tokio::net::TcpStream>, Error>(Some(s)),
								Err(e) => {
									error!("Failed to accept connection: {e:#}");
									Ok(None)
								}
							}
						} => {
							if let Some(s) = s {
								if let Some(tls) = tls.clone() {
									let router = router.clone();
									let watcher = graceful.watcher();
									tokio::spawn(async move {
										let handshake = timeout(TLS_HANDSHAKE_TIMEOUT, tls.accept(s));
										let tls_stream = match handshake.await {
											Ok(Ok(tls_stream)) => tls_stream,
											Ok(Err(err)) => {
												error!("failed to perform TLS handshake: {err:#}");
												return;
											}
											Err(_) => {
												error!("TLS handshake timed out");
												return;
											}
										};
										let io = TokioIo::new(tls_stream);
										let conn = http1::Builder::new().serve_connection(io, router);
										if let Err(e) = watcher.watch(conn).await {
											error!("API TLS server error: {:?}", e);
										}
									});
								} else {
									let io = TokioIo::new(s);
									let conn = http1::Builder::new().serve_connection(io, router.clone());
									let fut = graceful.watch(conn);
									tokio::spawn(async move {
										if let Err(e) = fut.await {
											error!("API HTTP server error: {:?}", e);
										}
									});
								};
							} else {
								continue;
							}
						}
						_ = &mut signal => {
							drop(l);
							break;
						}
					}
				}

				// Now start the shutdown and wait for them to complete
				// Also start a timeout to limit how long to wait.
				tokio::select! {
					_ = graceful.shutdown() => {
						warn!("API server gracefully stopped");
					},
					_ = sleep(GRACEFUL_SHUTDOWN_TIMEOUT) => {
						warn!("API server timed out wait for all connections to close");
					}
				}
			}
			Err(e) => {
				error!("API listener binding error: {}", e);
			}
		}
	};

	let rt = Runtime::new()
		.map_err(|e| error!("HTTP API server error: {}", e))
		.unwrap();
	rt.block_on(server);
}

/// Signal for graceful API server shutdown.
async fn shutdown_signal(mut rx: mpsc::Receiver<()>) {
	let _ = rx.recv().await;
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
