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

use failure::ResultExt;
use libwallet::types::*;
use std::collections::HashMap;
use std::io;

use futures::{Future, Stream};
use hyper;
use hyper::header::ContentType;
use hyper::{Method, Request};
use serde_json;
use tokio_core::reactor;

use api;
use error::{Error, ErrorKind};
use libtx::slate::Slate;
use util::secp::pedersen;
use util::{self, LOGGER};

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

/// Posts a transaction to a grin node
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

/// Retrieve outputs from node
pub fn get_outputs_from_node(
	addr: &str,
	wallet_outputs: Vec<pedersen::Commitment>,
) -> Result<HashMap<pedersen::Commitment, String>, Error> {
	// build the necessary query params -
	// ?id=xxx&id=yyy&id=zzz
	let query_params: Vec<String> = wallet_outputs
		.iter()
		.map(|commit| format!("id={}", util::to_hex(commit.as_ref().to_vec())))
		.collect();

	// build a map of api outputs by commit so we can look them up efficiently
	let mut api_outputs: HashMap<pedersen::Commitment, String> = HashMap::new();

	for query_chunk in query_params.chunks(1000) {
		let url = format!("{}/v1/chain/outputs/byids?{}", addr, query_chunk.join("&"),);

		match api::client::get::<Vec<api::Output>>(url.as_str()) {
			Ok(outputs) => for out in outputs {
				api_outputs.insert(out.commit.commit(), util::to_hex(out.commit.to_vec()));
			},
			Err(e) => {
				// if we got anything other than 200 back from server, don't attempt to refresh
				// the wallet data after
				return Err(e).context(ErrorKind::Node)?;
			}
		}
	}
	Ok(api_outputs)
}

/// Get any missing block hashes from node
pub fn get_missing_block_hashes_from_node(
	addr: &str,
	height: u64,
	wallet_outputs: Vec<pedersen::Commitment>,
) -> Result<
	(
		HashMap<pedersen::Commitment, (u64, BlockIdentifier)>,
		HashMap<pedersen::Commitment, MerkleProofWrapper>,
	),
	Error,
> {
	let id_params: Vec<String> = wallet_outputs
		.iter()
		.map(|commit| format!("id={}", util::to_hex(commit.as_ref().to_vec())))
		.collect();

	let height_params = [format!("start_height={}&end_height={}", 0, height)];
	let mut api_blocks: HashMap<pedersen::Commitment, (u64, BlockIdentifier)> = HashMap::new();
	let mut api_merkle_proofs: HashMap<pedersen::Commitment, MerkleProofWrapper> = HashMap::new();

	// Split up into separate requests, to avoid hitting http limits
	for mut query_chunk in id_params.chunks(1000) {
		let url = format!(
			"{}/v1/chain/outputs/byheight?{}",
			addr,
			[&height_params, query_chunk].concat().join("&"),
		);

		match api::client::get::<Vec<api::BlockOutputs>>(url.as_str()) {
			Ok(blocks) => for block in blocks {
				for out in block.outputs {
					api_blocks.insert(
						out.commit,
						(
							block.header.height,
							BlockIdentifier::from_hex(&block.header.hash).unwrap(),
						),
					);
					if let Some(merkle_proof) = out.merkle_proof {
						let wrapper = MerkleProofWrapper(merkle_proof);
						api_merkle_proofs.insert(out.commit, wrapper);
					}
				}
			},
			Err(e) => {
				// if we got anything other than 200 back from server, bye
				return Err(e).context(ErrorKind::Node)?;
			}
		}
	}
	Ok((api_blocks, api_merkle_proofs))
}
