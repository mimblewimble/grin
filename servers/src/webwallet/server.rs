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

//! Integrated static file server to serve up a pre-compiled web-wallet
//! application locally

use futures::{future, Async::*, Future, Poll};
use http::response::Builder as ResponseBuilder;
use http::{header, Request, Response, StatusCode};
use hyper::service::Service;
use hyper::{rt, Body, Server};
use hyper_staticfile::{Static, StaticFuture};
use std::env;
use std::io::Error;
use std::thread;

use util::LOGGER;

/// Future returned from `MainService`.
enum MainFuture {
	Root,
	Static(StaticFuture<Body>),
}

impl Future for MainFuture {
	type Item = Response<Body>;
	type Error = Error;

	fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
		match *self {
			MainFuture::Root => {
				let res = ResponseBuilder::new()
					.status(StatusCode::MOVED_PERMANENTLY)
					.header(header::LOCATION, "/index.html")
					.body(Body::empty())
					.expect("unable to build response");
				Ok(Ready(res))
			}
			MainFuture::Static(ref mut future) => future.poll(),
		}
	}
}

/// Hyper `Service` implementation that serves all requests.
struct MainService {
	static_: Static,
}

impl MainService {
	fn new() -> MainService {
		// Set up directory relative to executable for the time being
		let mut exe_path = env::current_exe().unwrap();
		exe_path.pop();
		exe_path.push("grin-wallet");
		MainService {
			static_: Static::new(exe_path),
		}
	}
}

impl Service for MainService {
	type ReqBody = Body;
	type ResBody = Body;
	type Error = Error;
	type Future = MainFuture;

	fn call(&mut self, req: Request<Body>) -> MainFuture {
		if req.uri().path() == "/" {
			MainFuture::Root
		} else {
			MainFuture::Static(self.static_.serve(req))
		}
	}
}

/// Start the webwallet server to serve up static files from the given
/// directory
pub fn start_webwallet_server() {
	let _ = thread::Builder::new()
		.name("webwallet_server".to_string())
		.spawn(move || {
			let addr = ([127, 0, 0, 1], 13421).into();
			let server = Server::bind(&addr)
				.serve(|| future::ok::<_, Error>(MainService::new()))
				.map_err(|e| eprintln!("server error: {}", e));
			warn!(
				LOGGER,
				"Grin Web-Wallet Application is running at http://{}/", addr
			);
			rt::run(server);
		});
}
