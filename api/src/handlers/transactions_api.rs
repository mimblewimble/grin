// Copyright 2021 The Grin Developers
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
// GET /v1/txhashset/heightstopmmr?start_height=1&end_height=1000
//
// Build a merkle proof for a given pos
// GET /v1/txhashset/merkleproof?n=1

pub struct TxHashSetHandler {
	pub chain: Weak<chain::Chain>,
}

impl TxHashSetHandler {
	// gets roots
	fn get_roots(&self) -> Result<TxHashSet, Error> {
		let chain = w(&self.chain)?;
		TxHashSet::from_head(&chain)
			.map_err(|e| Error::Internal(format!("failed to read roots from txhashset: {}", e)))
	}

	// gets last n outputs inserted in to the tree
	fn get_last_n_output(&self, distance: u64) -> Result<Vec<TxHashSetNode>, Error> {
		let chain = w(&self.chain)?;
		Ok(TxHashSetNode::get_last_n_output(&chain, distance))
	}

	// gets last n rangeproofs inserted in to the tree
	fn get_last_n_rangeproof(&self, distance: u64) -> Result<Vec<TxHashSetNode>, Error> {
		let chain = w(&self.chain)?;
		Ok(TxHashSetNode::get_last_n_rangeproof(&chain, distance))
	}

	// gets last n kernels inserted in to the tree
	fn get_last_n_kernel(&self, distance: u64) -> Result<Vec<TxHashSetNode>, Error> {
		let chain = w(&self.chain)?;
		Ok(TxHashSetNode::get_last_n_kernel(&chain, distance))
	}

	// allows traversal of utxo set
	fn outputs(
		&self,
		start_index: u64,
		end_index: Option<u64>,
		mut max: u64,
	) -> Result<OutputListing, Error> {
		//set a limit here
		if max > 10_000 {
			max = 10_000;
		}
		let chain = w(&self.chain)?;
		let outputs = chain
			.unspent_outputs_by_pmmr_index(start_index, max, end_index)
			.map_err(|_| Error::NotFound)?;
		let out = OutputListing {
			last_retrieved_index: outputs.0,
			highest_index: outputs.1,
			outputs: outputs
				.2
				.iter()
				.map(|x| OutputPrintable::from_output(x, &chain, None, true, true))
				.collect::<Result<Vec<_>, _>>()
				.map_err(|e| Error::Internal(format!("chain error: {}", e)))?,
		};
		Ok(out)
	}

	// allows traversal of utxo set bounded within a block range
	pub fn block_height_range_to_pmmr_indices(
		&self,
		start_block_height: u64,
		end_block_height: Option<u64>,
	) -> Result<OutputListing, Error> {
		let chain = w(&self.chain)?;
		let range = chain
			.block_height_range_to_pmmr_indices(start_block_height, end_block_height)
			.map_err(|_| Error::NotFound)?;
		let out = OutputListing {
			last_retrieved_index: range.0,
			highest_index: range.1,
			outputs: vec![],
		};
		Ok(out)
	}

	// return a dummy output with merkle proof for position filled out
	// (to avoid having to create a new type to pass around)
	fn get_merkle_proof_for_output(&self, id: &str) -> Result<OutputPrintable, Error> {
		let c = util::from_hex(id)
			.map_err(|_| Error::Argument(format!("Not a valid commitment: {}", id)))?;
		let commit = Commitment::from_vec(c);
		let chain = w(&self.chain)?;
		let output_pos = chain.get_output_pos(&commit).map_err(|_| Error::NotFound)?;
		let merkle_proof =
			chain::Chain::get_merkle_proof_for_pos(&chain, commit).map_err(|_| Error::NotFound)?;
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
		let end_index = match parse_param_no_err!(params, "end_index", 0) {
			0 => None,
			i => Some(i),
		};
		let max = parse_param_no_err!(params, "max", 100);
		let id = parse_param_no_err!(params, "id", "".to_owned());
		let start_height = parse_param_no_err!(params, "start_height", 1);
		let end_height = match parse_param_no_err!(params, "end_height", 0) {
			0 => None,
			h => Some(h),
		};

		match right_path_element!(req) {
			"roots" => result_to_response(self.get_roots()),
			"lastoutputs" => result_to_response(self.get_last_n_output(last_n)),
			"lastrangeproofs" => result_to_response(self.get_last_n_rangeproof(last_n)),
			"lastkernels" => result_to_response(self.get_last_n_kernel(last_n)),
			"outputs" => result_to_response(self.outputs(start_index, end_index, max)),
			"heightstopmmr" => result_to_response(
				self.block_height_range_to_pmmr_indices(start_height, end_height),
			),
			"merkleproof" => result_to_response(self.get_merkle_proof_for_output(&id)),
			_ => response(StatusCode::BAD_REQUEST, ""),
		}
	}
}
