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


use std::sync::Arc;

use iron::prelude::*;
use iron::Handler;
use iron::status;
use urlencoded::UrlEncodedQuery;
use serde_json;

use chain;
use rest::*;
use types::*;
use secp::pedersen::Commitment;
use util;
use util::LOGGER;


pub struct UtxoHandler {
	pub chain: Arc<chain::Chain>,
}

impl UtxoHandler {
	fn get_utxo(&self, id: &str) -> Result<Output, Error> {
		debug!(LOGGER, "getting utxo: {}", id);
		let c = util::from_hex(String::from(id))
			.map_err(|_| {
				Error::Argument(format!("Not a valid commitment: {}", id))
			})?;
		let commit = Commitment::from_vec(c);

		let out = self.chain.get_unspent(&commit)
			.map_err(|_| Error::NotFound)?;

		let header = self.chain
			.get_block_header_by_output_commit(&commit)
			.map_err(|_| Error::NotFound)?;

		Ok(Output::from_output(&out, &header))
	}
}

//
// Supports retrieval of multiple outputs in a single request -
// GET /v2/chain/utxos?id=xxx,yyy,zzz
// GET /v2/chain/utxos?id=xxx&id=yyy&id=zzz
//
impl Handler for UtxoHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let mut commitments: Vec<&str> = vec![];
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(ids) = params.get("id") {
				for id in ids {
					for id in id.split(",") {
						commitments.push(id.clone());
					}
				}
			}
		}

		let mut utxos: Vec<Output> = vec![];

		for commit in commitments {
			if let Ok(out) = self.get_utxo(commit) {
				utxos.push(out);
			}
		}

		match serde_json::to_string(&utxos) {
			Ok(json) => Ok(Response::with((status::Ok, json))),
			Err(_) => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}
