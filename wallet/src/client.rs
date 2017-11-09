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

use std::{io, time};
use std::ops::FnMut;

use futures::{Future, Stream};
use hyper;
use hyper::{Method, Request};
use hyper::header::ContentType;
use tokio_core::reactor;
use tokio_retry::Retry;
use tokio_retry::strategy::FibonacciBackoff;
use serde_json;

use types::*;
use util::LOGGER;

/// Call the wallet API to create a coinbase output for the given block_fees.
/// Will retry based on default "retry forever with backoff" behavior.
pub fn create_coinbase(url: &str, block_fees: &BlockFees) -> Result<CbData, Error> {
	retry_backoff_forever(|| {
		let res = single_create_coinbase(&url, &block_fees);
		if let Err(_) = res {
			error!(
				LOGGER,
				"Failed to get coinbase via wallet API (will retry)..."
			);
		}
		res
	})
}

/// Runs the specified function wrapped in some basic retry logic.
fn retry_backoff_forever<F, R>(f: F) -> Result<R, Error>
where
	F: FnMut() -> Result<R, Error>,
{
	let mut core = reactor::Core::new()?;
	let retry_strategy =
		FibonacciBackoff::from_millis(100).max_delay(time::Duration::from_secs(10));
	let retry_future = Retry::spawn(core.handle(), retry_strategy, f);
	let res = core.run(retry_future).unwrap();
	Ok(res)
}

pub fn send_partial_tx(url: &str, partial_tx: &PartialTx) -> Result<(), Error> {
	single_send_partial_tx(url, partial_tx)
}

fn single_send_partial_tx(url: &str, partial_tx: &PartialTx) -> Result<(), Error> {
	let mut core = reactor::Core::new()?;
	let client = hyper::Client::new(&core.handle());

	let mut req = Request::new(Method::Post, url.parse()?);
	req.headers_mut().set(ContentType::json());
	let json = serde_json::to_string(&partial_tx)?;
	req.set_body(json);

	let work = client.request(req);
	let _ = core.run(work).and_then(|res|{
		if res.status()==hyper::StatusCode::Ok {
			info!(LOGGER, "Transaction sent successfully");
		} else {
			error!(LOGGER, "Error sending transaction - status: {}", res.status());
		}
		Ok(())
	})?;
	Ok(())
}

/// Makes a single request to the wallet API to create a new coinbase output.
fn single_create_coinbase(url: &str, block_fees: &BlockFees) -> Result<CbData, Error> {
	let mut core = reactor::Core::new()?;
	let client = hyper::Client::new(&core.handle());

	let mut req = Request::new(Method::Post, url.parse()?);
	req.headers_mut().set(ContentType::json());
	let json = serde_json::to_string(&block_fees)?;
	req.set_body(json);

	let work = client.request(req).and_then(|res| {
		res.body().concat2().and_then(move |body| {
			let coinbase: CbData =
				serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
			Ok(coinbase)
		})
	});

	let res = core.run(work)?;
	Ok(res)
}
