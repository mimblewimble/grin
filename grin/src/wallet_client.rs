// Copyright 2017 The Grin Developers
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

use std::io;
use std::ops::FnMut;

use hyper;
use hyper::{Method, Request};
use hyper::header::ContentType;
use futures::{Future, Stream};
use tokio_core::reactor;
use tokio_retry::Retry;
use tokio_retry::strategy::FibonacciBackoff;
use serde_json;

use types::Error;
use wallet::{BlockFees, CbData};
use util::LOGGER;

/// Runs the specified function wrapped in some basic retry logic.
/// TODO - Feels like we should be able to pass the reactor handle into the function itself?
pub fn with_retry<F, R>(f: F) -> Result<R, Error>
	where F: (FnMut() -> Result<R, Error>)
{
	let mut core = reactor::Core::new()?;
	let retry_strategy = FibonacciBackoff::from_millis(250).take(3);
	let retry_future = Retry::spawn(core.handle(), retry_strategy, f);
	let res = core.run(retry_future).unwrap();
	Ok(res)
}

///
/// Call the wallet API to create a coinbase output for the given block_fees.
///
pub fn create_coinbase(url: &str, block_fees: &BlockFees) -> Result<CbData, Error> {
	debug!(LOGGER, "Calling wallet API to create a new coinbase output");
	let mut core = reactor::Core::new()?;
	let client = hyper::Client::new(&core.handle());

	let mut req = Request::new(Method::Post, url.parse()?);
	req.headers_mut().set(ContentType::json());
	let json = serde_json::to_string(&block_fees)?;
	req.set_body(json);

	let work = client.request(req).and_then(|res| {
		res.body().concat2().and_then(move |body| {
			let coinbase: CbData = serde_json::from_slice(&body)
				.map_err(|e| {io::Error::new(io::ErrorKind::Other, e)})?;
			Ok(coinbase)
		})
	});

	let res = core.run(work)?;
	Ok(res)
}
