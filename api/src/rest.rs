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

use std::fmt::{self, Display, Formatter};
use std::mem;
use std::net::ToSocketAddrs;
use std::string::ToString;

use failure::{Backtrace, Context, Fail, ResultExt};
use iron::middleware::Handler;
use iron::prelude::*;
use iron::{status, Listening};
use mount::Mount;
use router::Router;

use store;

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
pub struct ApiServer {
	root: String,
	router: Router,
	mount: Mount,
	server_listener: Option<Listening>,
}

impl ApiServer {
	/// Creates a new ApiServer that will serve ApiEndpoint implementations
	/// under the root URL.
	pub fn new(root: String) -> ApiServer {
		ApiServer {
			root: root,
			router: Router::new(),
			mount: Mount::new(),
			server_listener: None,
		}
	}

	/// Starts the ApiServer at the provided address.
	pub fn start<A: ToSocketAddrs>(&mut self, addr: A) -> Result<(), String> {
		// replace this value to satisfy borrow checker
		let r = mem::replace(&mut self.router, Router::new());
		let mut m = mem::replace(&mut self.mount, Mount::new());
		m.mount("/", r);
		let result = Iron::new(m).http(addr);
		let return_value = result.as_ref().map(|_| ()).map_err(|e| e.to_string());
		self.server_listener = Some(result.unwrap());
		return_value
	}

	/// Stops the API server
	pub fn stop(&mut self) {
		let r = mem::replace(&mut self.server_listener, None);
		r.unwrap().close().unwrap();
	}

	/// Registers an iron handler (via mount)
	pub fn register_handler<H: Handler>(&mut self, handler: H) -> &mut Mount {
		self.mount.mount(&self.root, handler)
	}
}
