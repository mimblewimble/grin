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

// Sum tree handler

pub struct SumTreeHandler {
	pub chain: Arc<chain::Chain>,
}

impl SumTreeHandler {
	//gets roots
	fn get_roots(&self) -> SumTrees {
		SumTrees::from_head(self.chain.clone())
	}

	// gets last n utxos inserted in to the tree
	fn get_last_n_utxo(&self, distance:u64) -> Vec<SumTreeNode> {
		SumTreeNode::get_last_n_utxo(self.chain.clone(), distance)
	}

	// gets last n utxos inserted in to the tree
	fn get_last_n_rangeproof(&self, distance:u64) -> Vec<SumTreeNode> {
		SumTreeNode::get_last_n_rangeproof(self.chain.clone(), distance)
	}

	// gets last n utxos inserted in to the tree
	fn get_last_n_kernel(&self, distance:u64) -> Vec<SumTreeNode> {
		SumTreeNode::get_last_n_kernel(self.chain.clone(), distance)
	}

}

//
// Retrieve the roots:
// GET /v2/sumtrees/roots
//
// Last inserted nodes::
// GET /v2/sumtrees/lastutxos (gets last 10)
// GET /v2/sumtrees/lastutxos?n=5
// GET /v2/sumtrees/lastrangeproofs
// GET /v2/sumtrees/lastkernels
//

impl Handler for SumTreeHandler {
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let url = req.url.clone();
		let mut path_elems = url.path();
		if *path_elems.last().unwrap() == "" {
			path_elems.pop();
		}
		//TODO: probably need to set a reasonable max limit here
		let mut last_n=10;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(nums) = params.get("n") {
				for num in nums {
					if let Ok(n) = str::parse(num) {
						last_n=n;
					}
				}
			}
		}
		match *path_elems.last().unwrap(){
			"roots" => match serde_json::to_string_pretty(&self.get_roots()) {
				Ok(json) => Ok(Response::with((status::Ok, json))),
				Err(_) => Ok(Response::with((status::BadRequest, ""))),
			},
			"lastutxos" => match serde_json::to_string_pretty(&self.get_last_n_utxo(last_n)) {
				Ok(json) => Ok(Response::with((status::Ok, json))),
				Err(_) => Ok(Response::with((status::BadRequest, ""))),
			},
			"lastrangeproofs" => match serde_json::to_string_pretty(&self.get_last_n_rangeproof(last_n)) {
				Ok(json) => Ok(Response::with((status::Ok, json))),
				Err(_) => Ok(Response::with((status::BadRequest, ""))),
			},
			"lastkernels" => match serde_json::to_string_pretty(&self.get_last_n_kernel(last_n)) {
				Ok(json) => Ok(Response::with((status::Ok, json))),
				Err(_) => Ok(Response::with((status::BadRequest, ""))),
			},_ => Ok(Response::with((status::BadRequest, "")))
		}
	}
}

// Chain Handler

pub struct ChainHandler {
	pub chain: Arc<chain::Chain>,
}

impl ChainHandler {
	fn get_tip(&self) -> Tip {
		Tip::from_tip(self.chain.head().unwrap())
	}
}

//
// Get the head details
// GET /v2/chain
//

impl Handler for ChainHandler {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		match serde_json::to_string_pretty(&self.get_tip()) {
			Ok(json) => Ok(Response::with((status::Ok, json))),
			Err(_) => Ok(Response::with((status::BadRequest, ""))),
		}
	}
}
