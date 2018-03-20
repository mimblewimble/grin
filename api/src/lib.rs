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
//! # Examples
//!
//! ```text
//!	struct IndexHandler {
//!	 list: Vec<String>,
//!	}
//!
//!	impl Handler for IndexHandler {
//!	 fn handle(&self, _req: Request, _params: PathParams) -> Result<Response, HyperError> {
//!		some implementation...
//!	 }
//!	}
//!
//!	let router = router!(
//!	 get "/v1" => index_handler,
//!	);
//!
//!	let apis = ApiServer::new(router.unwrap());
//!	info!(LOGGER, "Starting Http API server at {}.", addr);
//!	let socket_addr = addr[..].parse().unwrap(); 
//!	let server = Http::new().bind(&socket_addr, apis).unwrap();
//!	server.run().unwrap();
//!
//! ```

extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate grin_pool as pool;
extern crate grin_store as store;
extern crate grin_util as util;

#[macro_use]
extern crate error_chain;
extern crate fnv;
extern crate futures;
extern crate hyper;
#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate tokio_core;
extern crate url;

pub mod client;
#[macro_use]
pub mod router;
mod handlers;
pub mod rest;
mod types;

pub use handlers::start_rest_apis;
pub use types::*;
pub use rest::*;
pub use router::router::Router;
