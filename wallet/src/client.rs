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

use failure::ResultExt;
use futures::{Future, Stream};
use hyper;
use hyper::header::ContentType;
use hyper::{Method, Request};
use libwallet::transaction::Slate;
use serde_json;
use tokio_core::reactor;

use std::io;
use types::*;
use util::LOGGER;

/// Call the wallet API to create a coinbase output for the given block_fees.
/// Will retry based on default "retry forever with backoff" behavior.
pub fn create_coinbase(url: &str, block_fees: &BlockFees) -> Result<CbData, Error> {
	match single_create_coinbase(&url, &block_fees) {
		Err(e) => {
			error!(
				LOGGER,
				"Failed to get coinbase from {}. Run grin wallet listen", url
			);
			Err(e)
		}
		Ok(res) => Ok(res),
	}
}

pub fn send_slate(url: &str, slate: &Slate, fluff: bool) -> Result<Slate, Error> {
	let mut core = reactor::Core::new().context(ErrorKind::Hyper)?;
	let client = hyper::Client::new(&core.handle());

	// In case we want to do an express send
	let mut url_pool = url.to_owned();
	if fluff {
		url_pool = format!("{}{}", url, "?fluff");
	}

	let mut req = Request::new(
		Method::Post,
		url_pool.parse::<hyper::Uri>().context(ErrorKind::Hyper)?,
	);
	req.headers_mut().set(ContentType::json());
	let json = serde_json::to_string(&slate).context(ErrorKind::Hyper)?;
	req.set_body(json);

	let work = client.request(req).and_then(|res| {
		res.body().concat2().and_then(move |body| {
			let slate: Slate =
				serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
			Ok(slate)
		})
	});
	let res = core.run(work).context(ErrorKind::Hyper)?;
	Ok(res)
}

/// Makes a single request to the wallet API to create a new coinbase output.
fn single_create_coinbase(url: &str, block_fees: &BlockFees) -> Result<CbData, Error> {
	let mut core =
		reactor::Core::new().context(ErrorKind::GenericError("Could not create reactor"))?;
	let client = hyper::Client::new(&core.handle());

	let mut req = Request::new(
		Method::Post,
		url.parse::<hyper::Uri>().context(ErrorKind::Uri)?,
	);
	req.headers_mut().set(ContentType::json());
	let json = serde_json::to_string(&block_fees).context(ErrorKind::Format)?;
	req.set_body(json);

	let work = client.request(req).and_then(|res| {
		res.body().concat2().and_then(move |body| {
			let coinbase: CbData =
				serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
			Ok(coinbase)
		})
	});

	let res = core.run(work)
		.context(ErrorKind::GenericError("Could not run core"))?;
	Ok(res)
}
