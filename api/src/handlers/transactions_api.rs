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

use super::utils::w;
use crate::chain;
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::util;
use crate::util::secp::pedersen::Commitment;
use crate::web::*;
use failure::ResultExt;
use hyper::{Body, Request, StatusCode};
use std::sync::Weak;

// Sum tree handler. Retrieve the roots:
// GET /v1/txhashset/roots
//
// Last inserted nodes::
// GET /v1/txhashset/lastoutputs (gets last 10)
// GET /v1/txhashset/lastoutputs?n=5
// GET /v1/txhashset/lastrangeproofs
// GET /v1/txhashset/lastkernels

// UTXO traversal::
// GET /v1/txhashset/outputs?start_index=1&max=100
//
// Build a merkle proof for a given pos
// GET /v1/txhashset/merkleproof?n=1

pub struct TxHashSetHandler {
	pub chain: Weak<chain::Chain>,
}

impl TxHashSetHandler {
	// gets roots
	fn get_roots(&self) -> TxHashSet {
		TxHashSet::from_head(w(&self.chain))
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_output(&self, distance: u64) -> Vec<TxHashSetNode> {
		TxHashSetNode::get_last_n_output(w(&self.chain), distance)
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_rangeproof(&self, distance: u64) -> Vec<TxHashSetNode> {
		TxHashSetNode::get_last_n_rangeproof(w(&self.chain), distance)
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_kernel(&self, distance: u64) -> Vec<TxHashSetNode> {
		TxHashSetNode::get_last_n_kernel(w(&self.chain), distance)
	}

	// allows traversal of utxo set
	fn outputs(&self, start_index: u64, mut max: u64) -> Result<OutputListing, Error> {
		//set a limit here
		if max > 1000 {
			max = 1000;
		}
		let outputs = w(&self.chain)
			.unspent_outputs_by_insertion_index(start_index, max)
			.context(ErrorKind::NotFound)?;
		Ok(OutputListing {
			last_retrieved_index: outputs.0,
			highest_index: outputs.1,
			outputs: outputs
				.2
				.iter()
				.map(|x| OutputPrintable::from_output(x, w(&self.chain), None, true))
				.collect(),
		})
	}

	// return a dummy output with merkle proof for position filled out
	// (to avoid having to create a new type to pass around)
	fn get_merkle_proof_for_output(&self, id: &str) -> Result<OutputPrintable, Error> {
		let c = util::from_hex(String::from(id)).context(ErrorKind::Argument(format!(
			"Not a valid commitment: {}",
			id
		)))?;
		let commit = Commitment::from_vec(c);
		let output_pos = w(&self.chain)
			.get_output_pos(&commit)
			.context(ErrorKind::NotFound)?;
		let merkle_proof = chain::Chain::get_merkle_proof_for_pos(&w(&self.chain), commit)
			.map_err(|_| ErrorKind::NotFound)?;
		Ok(OutputPrintable {
			output_type: OutputType::Coinbase,
			commit: Commitment::from_vec(vec![]),
			spent: false,
			proof: None,
			proof_hash: "".to_string(),
			block_height: None,
			merkle_proof: Some(merkle_proof),
			mmr_index: output_pos,
		})
	}
}

impl Handler for TxHashSetHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		// TODO: probably need to set a reasonable max limit here
		let params = QueryParams::from(req.uri().query());
		let last_n = parse_param_no_err!(params, "n", 10);
		let start_index = parse_param_no_err!(params, "start_index", 1);
		let max = parse_param_no_err!(params, "max", 100);
		let id = parse_param_no_err!(params, "id", "".to_owned());

		match right_path_element!(req) {
			"roots" => json_response_pretty(&self.get_roots()),
			"lastoutputs" => json_response_pretty(&self.get_last_n_output(last_n)),
			"lastrangeproofs" => json_response_pretty(&self.get_last_n_rangeproof(last_n)),
			"lastkernels" => json_response_pretty(&self.get_last_n_kernel(last_n)),
			"outputs" => result_to_response(self.outputs(start_index, max)),
			"merkleproof" => result_to_response(self.get_merkle_proof_for_output(&id)),
			_ => response(StatusCode::BAD_REQUEST, ""),
		}
	}
}
