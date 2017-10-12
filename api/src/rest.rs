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
use std::fmt::{self, Display, Debug, Formatter};
use std::io::Read;
use std::net::ToSocketAddrs;
use std::string::ToString;
use std::str::FromStr;
use std::mem;

use iron::{Iron, Request, Response, IronResult, IronError, status, headers, Listening};
use iron::method::Method;
use iron::modifiers::Header;
use iron::middleware::Handler;
use router::Router;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json;

use store;
use util::LOGGER;

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

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Operation {
	Create,
	Delete,
	Update,
	Get,
	Custom(String),
}

impl Operation {
	fn to_method(&self) -> Method {
		match *self {
			Operation::Create => Method::Post,
			Operation::Delete => Method::Delete,
			Operation::Update => Method::Put,
			Operation::Get => Method::Get,
			Operation::Custom(_) => Method::Post,
		}
	}
}

pub type ApiResult<T> = ::std::result::Result<T, Error>;

/// Trait to implement to expose a service as a RESTful HTTP endpoint. Each
/// method corresponds to a specific relative URL and HTTP method following
/// basic REST principles:
///
/// * create: POST /
/// * get:    GET /:id
/// * update: PUT /:id
/// * delete: DELETE /:id
///
/// The methods method defines which operation the endpoint implements, they're
/// all optional by default. It also allows the framework to automatically
/// define the OPTIONS HTTP method.
///
/// The type accepted by create and update, and returned by get, must implement
/// the serde Serialize and Deserialize traits. The identifier type returned by
/// create and accepted by all other methods must have a string representation.
pub trait ApiEndpoint: Clone + Send + Sync + 'static {
	type ID: ToString + FromStr;
	type T: Serialize + DeserializeOwned;
	type OP_IN: Serialize + DeserializeOwned;
	type OP_OUT: Serialize + DeserializeOwned;

	fn operations(&self) -> Vec<Operation>;

	#[allow(unused_variables)]
	fn create(&self, o: Self::T) -> ApiResult<Self::ID> {
		unimplemented!()
	}

	#[allow(unused_variables)]
	fn delete(&self, id: Self::ID) -> ApiResult<()> {
		unimplemented!()
	}

	#[allow(unused_variables)]
	fn update(&self, id: Self::ID, o: Self::T) -> ApiResult<()> {
		unimplemented!()
	}

	#[allow(unused_variables)]
	fn get(&self, id: Self::ID) -> ApiResult<Self::T> {
		unimplemented!()
	}

	#[allow(unused_variables)]
	fn operation(&self, op: String, input: Self::OP_IN) -> ApiResult<Self::OP_OUT> {
		unimplemented!()
	}
}

// Wrapper required to define the implementation below, Rust doesn't let us
// define the parametric implementation for trait from another crate.
struct ApiWrapper<E>(E);

impl<E> Handler for ApiWrapper<E>
	where E: ApiEndpoint,
	      <<E as ApiEndpoint>::ID as FromStr>::Err: Debug + Send + error::Error
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		match req.method {
			Method::Get => {
				let res = self.0.get(extract_param(req, "id")?)?;
				let res_json = serde_json::to_string(&res)
					.map_err(|e| IronError::new(e, status::InternalServerError))?;
				Ok(Response::with((status::Ok, res_json)))
			}
			Method::Put => {
				let id = extract_param(req, "id")?;
				let t: E::T = serde_json::from_reader(req.body.by_ref())
					.map_err(|e| IronError::new(e, status::BadRequest))?;
				self.0.update(id, t)?;
				Ok(Response::with(status::NoContent))
			}
			Method::Delete => {
				let id = extract_param(req, "id")?;
				self.0.delete(id)?;
				Ok(Response::with(status::NoContent))
			}
			Method::Post => {
				let t: E::T = serde_json::from_reader(req.body.by_ref())
					.map_err(|e| IronError::new(e, status::BadRequest))?;
				let id = self.0.create(t)?;
				Ok(Response::with((status::Created, id.to_string())))
			}
			_ => Ok(Response::with(status::MethodNotAllowed)),
		}
	}
}

struct OpWrapper<E> {
	operation: String,
	endpoint: E,
}

impl<E> Handler for OpWrapper<E>
    where E: ApiEndpoint
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let t: E::OP_IN = serde_json::from_reader(req.body.by_ref()).map_err(|e| {
			IronError::new(e, status::BadRequest)
		})?;
		let res = self.endpoint.operation(self.operation.clone(), t);
		match res {
			Ok(resp) => {
				let res_json = serde_json::to_string(&resp).map_err(|e| {
					IronError::new(e, status::InternalServerError)
				})?;
				Ok(Response::with((status::Ok, res_json)))
			}
			Err(e) => {
				error!(LOGGER, "API operation: {:?}", e);
				Err(IronError::from(e))
			}
		}
	}
}

fn extract_param<ID>(req: &mut Request, param: &'static str) -> IronResult<ID>
	where ID: ToString + FromStr,
	      <ID as FromStr>::Err: Debug + Send + error::Error + 'static
{

	let id = req.extensions
		.get::<Router>()
		.unwrap()
		.find(param)
		.unwrap_or("");
	id.parse::<ID>()
		.map_err(|e| IronError::new(e, status::BadRequest))
}

/// HTTP server allowing the registration of ApiEndpoint implementations.
pub struct ApiServer {
	root: String,
	router: Router,
	server_listener: Option<Listening>,
}

impl ApiServer {
	/// Creates a new ApiServer that will serve ApiEndpoint implementations
	/// under the root URL.
	pub fn new(root: String) -> ApiServer {
		ApiServer {
			root: root,
			router: Router::new(),
			server_listener: None,
		}
	}

	/// Starts the ApiServer at the provided address.
	pub fn start<A: ToSocketAddrs>(&mut self, addr: A) -> Result<(), String> {
		// replace this value to satisfy borrow checker
		let r = mem::replace(&mut self.router, Router::new());
		let result = Iron::new(r).http(addr);
		let return_value = result.as_ref().map(|_| ()).map_err(|e| e.to_string());
		self.server_listener = Some(result.unwrap());
		return_value
	}

	/// Stops the API server
	pub fn stop(&mut self) {
		let r = mem::replace(&mut self.server_listener, None);
		r.unwrap().close().unwrap();
	}

	/// Register a new API endpoint, providing a relative URL for the new
	/// endpoint.
	pub fn register_endpoint<E>(&mut self, subpath: String, endpoint: E)
		where E: ApiEndpoint,
		      <<E as ApiEndpoint>::ID as FromStr>::Err: Debug + Send + error::Error
	{

		assert_eq!(subpath.chars().nth(0).unwrap(), '/');

		// declare a route for each method actually implemented by the endpoint
		let route_postfix = &subpath[1..];
		let root = self.root.clone() + &subpath;
		for op in endpoint.operations() {
			let route_name = format!("{:?}_{}", op, route_postfix);

			// special case of custom operations
			if let Operation::Custom(op_s) = op.clone() {
				let wrapper = OpWrapper {
					operation: op_s.clone(),
					endpoint: endpoint.clone(),
				};
				let full_path = format!("{}/{}", root.clone(), op_s.clone());
				self.router
					.route(op.to_method(), full_path.clone(), wrapper, route_name);
				info!(LOGGER, "route: POST {}", full_path);
			} else {

				// regular REST operations
				let full_path = match op.clone() {
					Operation::Get => root.clone() + "/:id",
					Operation::Update => root.clone() + "/:id",
					Operation::Delete => root.clone() + "/:id",
					Operation::Create => root.clone(),
					_ => panic!("unreachable"),
				};
				let wrapper = ApiWrapper(endpoint.clone());
				self.router
					.route(op.to_method(), full_path.clone(), wrapper, route_name);
				info!(LOGGER, "route: {} {}", op.to_method(), full_path);
			}
		}

		// support for the HTTP Options method by differentiating what's on the
		// root resource vs the id resource
		let (root_opts, sub_opts) = endpoint
			.operations()
			.iter()
			.fold((vec![], vec![]), |mut acc, op| {
				let m = op.to_method();
				if m == Method::Post {
					acc.0.push(m);
				} else {
					acc.1.push(m);
				}
				acc
			});
		self.router.options(
			root.clone(),
			move |_: &mut Request| {
				Ok(Response::with((status::Ok, Header(headers::Allow(root_opts.clone())))))
			},
			"option_".to_string() + route_postfix,
		);
		self.router.options(
			root.clone() + "/:id",
			move |_: &mut Request| {
				Ok(Response::with((status::Ok, Header(headers::Allow(sub_opts.clone())))))
			},
			"option_id_".to_string() + route_postfix,
		);
	}
}


#[cfg(test)]
mod test {
	use super::*;

	#[derive(Serialize, Deserialize)]
	pub struct Animal {
		name: String,
		legs: u32,
		lethal: bool,
	}

	#[derive(Clone)]
	pub struct TestApi;

	impl ApiEndpoint for TestApi {
		type ID = String;
		type T = Animal;
		type OP_IN = ();
		type OP_OUT = ();

		fn operations(&self) -> Vec<Operation> {
			vec![Operation::Get]
		}

		fn get(&self, name: String) -> ApiResult<Animal> {
			Ok(Animal {
				name: name,
				legs: 4,
				lethal: false,
			})
		}
	}

	#[test]
	fn req_chain_json() {
		let mut apis = ApiServer::new("/v1".to_string());
		apis.register_endpoint("/animal".to_string(), TestApi);
	}
}
