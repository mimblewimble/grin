// Copyright 2016 The Grin Developers
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

use std::error;
use std::fmt::{self, Display, Formatter};
use std::net::ToSocketAddrs;
use std::string::ToString;
use std::mem;

use iron::prelude::*;
use iron::{status, Listening};
use iron::middleware::Handler;
use router::Router;
use mount::Mount;

use store;

/// Errors that can be returned by an ApiEndpoint implementation.
#[derive(Debug)]
pub enum Error {
	Internal(String),
	Argument(String),
	NotFound,
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter) -> fmt::Result {
		match *self {
			Error::Argument(ref s) => write!(f, "Bad arguments: {}", s),
			Error::Internal(ref s) => write!(f, "Internal error: {}", s),
			Error::NotFound => write!(f, "Not found."),
		}
	}
}

impl error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			Error::Argument(_) => "Bad arguments.",
			Error::Internal(_) => "Internal error.",
			Error::NotFound => "Not found.",
		}
	}
}

impl From<Error> for IronError {
	fn from(e: Error) -> IronError {
		match e {
			Error::Argument(_) => IronError::new(e, status::Status::BadRequest),
			Error::Internal(_) => IronError::new(e, status::Status::InternalServerError),
			Error::NotFound => IronError::new(e, status::Status::NotFound),
		}
	}
}

impl From<store::Error> for Error {
	fn from(e: store::Error) -> Error {
		match e {
			store::Error::NotFoundErr => Error::NotFound,
			_ => Error::Internal(e.to_string()),
		}
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
