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

use fnv::FnvHashMap;

use futures::IntoFuture;
use futures::future::FutureResult;

use hyper;
use hyper::{Error as HyperError, StatusCode};
use hyper::header::ContentLength;
use hyper::server::{Service, NewService, Request, Response};

use serde::Serialize;
use serde_json;

use std::error;
use std::fmt::{self, Display, Formatter};
use std::string::ToString;
use std::sync::Arc;

use super::router::Router;
use store;

/// Errors that can be returned by Handler implementation.
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

impl From<store::Error> for Error {
	fn from(e: store::Error) -> Error {
		match e {
			store::Error::NotFoundErr => Error::NotFound,
			_ => Error::Internal(e.to_string()),
		}
	}
}

// === Handler trait and associated type parameter ===

/// Type of path parameter.
///
/// The value of path parameter (a.k.a. wildcard) is retrieved and returned by
/// lookup function of node of radix trie.
/// For speed, FnvHashMap is used.
pub type PathParams = FnvHashMap<String, String>;

/// Handler trait
pub trait Handler: Send + Sync + 'static {
    /// Produce a `Response` from a Request, with the possibility of error.
    fn handle(&self, req: Request, params: PathParams) -> Result<Response, HyperError>;
}

/// Handler Fn implementation
impl<F> Handler for F
where
    F: Fn(Request, PathParams) -> Result<Response, HyperError> + Send + Sync + 'static,
{
    fn handle(&self, req: Request, params: PathParams) -> Result<Response, HyperError> {
        (*self)(req, params)
    }
}

/// API server implementing hyper::NewService trait
/// to makes copy of hyper::Service for each Http request.
pub struct ApiServer {
	service: Arc<ApiService>,
}

impl ApiServer {
	/// Creates a new ApiServer that will serve hyper::NewService implementation.
	pub fn new(router: Router) -> ApiServer {
		ApiServer {
			service: Arc::new(ApiService::new(router)),
		}
	}
}

impl NewService for ApiServer {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Instance = Arc<ApiService>;

    fn new_service(&self) -> ::std::io::Result<Self::Instance> {
		Ok(self.service.clone())
    }
}

/// API service implementing hyper::Service trait
/// to serve each Http request.
#[derive(Clone)]
pub struct ApiService {
	router: Arc<Router>
}

impl ApiService {
	/// Creates a new ApiService that will serve hyper::Service implementation.
	pub fn new(router: Router) -> ApiService {
		ApiService {
			router: Arc::new(router),
		}
	}
}

impl Service for ApiService {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = FutureResult<Response, hyper::Error>;

    fn call(&self, req: Self::Request) -> Self::Future {
		self.router.lookup(req).into_future()
    }
}

// === utils ===

/// Utility to serialize a struct into JSON and produce Response out of it.
pub fn json_response<T>(s: &T) -> Result<Response, HyperError>
where
	T: Serialize,
{
	match serde_json::to_string(s) {
        Ok(json) => Ok(Response::new()
                    .with_header(ContentLength(json.len() as u64))
                    .with_body(json)),
        Err(_) => error_response(Error::Internal("failed to serialize data into json.".to_string())),

	}
}

/// pretty-printed version of above
pub fn json_response_pretty<T>(s: &T) -> Result<Response, HyperError>
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
        Ok(json) => Ok(Response::new()
                    .with_header(ContentLength(json.len() as u64))
                    .with_body(json)),
        Err(_) => error_response(Error::Internal("failed to serialize data into pretty-printed json.".to_string())),

	}
}

/// Takes a `Error` enum as an argument and
/// returns Response.
pub fn error_response(e: Error) -> Result<Response, HyperError> {
    match e {
        Error::Argument(message) => Ok(Response::new()
                                .with_status(StatusCode::BadRequest)
                                .with_header(ContentLength(message.len() as u64))
                                .with_body(message)),
        Error::Internal(message) => Ok(Response::new()
                                .with_status(StatusCode::InternalServerError)
                                .with_header(ContentLength(message.len() as u64))
                                .with_body(message)),
        Error::NotFound => Ok(Response::new()
                                .with_status(StatusCode::NotFound)),
    }
}
