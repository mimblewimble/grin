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

//! Client functions, implementations of the WalletClient trait
//! specific to the FileWallet

use api;
use failure::ResultExt;
use futures::{Future, Stream};
use hyper;
use hyper::header::ContentType;
use hyper::{Method, Request};
use libtx::slate::Slate;
use serde_json;
use tokio_core::reactor;

use error::{Error, ErrorKind};
use libwallet::types::*;
use std::io;
use util::LOGGER;

/// Call the wallet API to create a coinbase output for the given block_fees.
/// Will retry based on default "retry forever with backoff" behavior.
pub fn create_coinbase(dest: &str, block_fees: &BlockFees) -> Result<CbData, Error> {
	let url = format!("{}/v1/wallet/foreign/build_coinbase", dest);
	match single_create_coinbase(&url, &block_fees) {
		Err(e) => {
			error!(
				LOGGER,
				"Failed to get coinbase from {}. Run grin wallet listen?", url
			);
			error!(LOGGER, "Underlying Error: {}", e.cause().unwrap());
			error!(LOGGER, "Backtrace: {}", e.backtrace().unwrap());
			Err(e)
		}
		Ok(res) => Ok(res),
	}
}

/// Send the slate to a listening wallet instance
pub fn send_tx_slate(dest: &str, slate: &Slate) -> Result<Slate, Error> {
	if &dest[..4] != "http" {
		error!(
			LOGGER,
			"dest formatted as {} but send -d expected stdout or http://IP:port", dest
		);
		Err(ErrorKind::Node)?
	}
	let url = format!("{}/v1/wallet/foreign/receive_tx", dest);
	debug!(LOGGER, "Posting transaction slate to {}", url);

	let mut core = reactor::Core::new().context(ErrorKind::Hyper)?;
	let client = hyper::Client::new(&core.handle());

	let url_pool = url.to_owned();

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
	trace!(LOGGER, "Sending coinbase request: {:?}", json);
	req.set_body(json);

	let work = client.request(req).and_then(|res| {
		res.body().concat2().and_then(move |body| {
			trace!(LOGGER, "Returned Body: {:?}", body);
			let coinbase: CbData =
				serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
			Ok(coinbase)
		})
	});

	let res = core.run(work)
		.context(ErrorKind::GenericError("Could not run core"))?;
	Ok(res)
}

/// Posts a tranaction to a grin node
pub fn post_tx(dest: &str, tx: &TxWrapper, fluff: bool) -> Result<(), Error> {
	let url;
	if fluff {
		url = format!("{}/v1/pool/push?fluff", dest);
	} else {
		url = format!("{}/v1/pool/push", dest);
	}
	let res = api::client::post(url.as_str(), tx).context(ErrorKind::Node)?;
	Ok(res)
}

/// Return the chain tip from a given node
pub fn get_chain_height(addr: &str) -> Result<u64, Error> {
	let url = format!("{}/v1/chain", addr);
	let res = api::client::get::<api::Tip>(url.as_str()).context(ErrorKind::Node)?;
	Ok(res.height)
}
