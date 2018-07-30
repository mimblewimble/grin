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

use hyper::rt::Future;
use hyper::service::service_fn;
use hyper::{Body, Request, Server};
use router::ResponseFuture;
use std::fmt::{self, Display};
use std::net::SocketAddr;
use tokio::runtime::current_thread::Runtime;

use failure::{Backtrace, Context, Fail};

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

/// HTTP server allowing the registration of ApiEndpoint implementations.
pub struct ApiServer {}

impl ApiServer {
	/// Creates a new ApiServer that will serve ApiEndpoint implementations
	/// under the root URL.
	pub fn new() -> ApiServer {
		ApiServer {}
	}

	/// Starts the ApiServer at the provided address.
	pub fn start<F>(&mut self, addr: SocketAddr, f: &'static F) -> Result<(), String>
	where
		F: Fn(Request<Body>) -> ResponseFuture + Send + Sync + 'static,
	{
		let server = Server::bind(&addr)
			.serve(move || service_fn(f))
			.map_err(|e| eprintln!("server error: {}", e));

		let mut rt = Runtime::new().unwrap();
		if rt.block_on(server).is_err() {
			return Err("tokio block_on error".to_owned());
		}
		Ok(())
	}

	/// Stops the API server
	pub fn stop(&mut self) {
		// TODO implement proper stop, the following method doesn't
		// work for current_thread runtime.
		//	if let Some(rt) = self.rt.take() {
		//		rt.shutdown_now().wait().unwrap();
		//	}
	}
}
