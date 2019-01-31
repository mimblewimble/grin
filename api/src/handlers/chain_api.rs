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

use super::utils::{get_output, w};
use crate::chain;
use crate::core::core::hash::Hashed;
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::util;
use crate::util::secp::pedersen::Commitment;
use crate::web::*;
use hyper::{Body, Request, StatusCode};
use std::collections::HashMap;
use std::sync::Weak;
use url::form_urlencoded;

/// Chain handler. Get the head details.
/// GET /v1/chain
pub struct ChainHandler {
	pub chain: Weak<chain::Chain>,
}

impl ChainHandler {
	fn get_tip(&self) -> Result<Tip, Error> {
		let head = w(&self.chain)
			.head()
			.map_err(|e| ErrorKind::Internal(format!("can't get head: {}", e)))?;
		Ok(Tip::from_tip(head))
	}
}

impl Handler for ChainHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_tip())
	}
}

/// Chain validation handler.
/// GET /v1/chain/validate
pub struct ChainValidationHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainValidationHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		match w(&self.chain).validate(true) {
			Ok(_) => response(StatusCode::OK, ""),
			Err(e) => response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("validate failed: {}", e),
			),
		}
	}
}

/// Chain compaction handler. Trigger a compaction of the chain state to regain
/// storage space.
/// POST /v1/chain/compact
pub struct ChainCompactHandler {
	pub chain: Weak<chain::Chain>,
}

impl Handler for ChainCompactHandler {
	fn post(&self, _req: Request<Body>) -> ResponseFuture {
		match w(&self.chain).compact() {
			Ok(_) => response(StatusCode::OK, ""),
			Err(e) => response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("compact failed: {}", e),
			),
		}
	}
}

// Supports retrieval of multiple outputs in a single request -
// GET /v1/chain/outputs/byids?id=xxx,yyy,zzz
// GET /v1/chain/outputs/byids?id=xxx&id=yyy&id=zzz
// GET /v1/chain/outputs/byheight?start_height=101&end_height=200
pub struct OutputHandler {
	pub chain: Weak<chain::Chain>,
}

impl OutputHandler {
	fn get_output(&self, id: &str) -> Result<Output, Error> {
		let res = get_output(&self.chain, id)?;
		Ok(res.0)
	}

	fn outputs_by_ids(&self, req: &Request<Body>) -> Result<Vec<Output>, Error> {
		let mut commitments: Vec<String> = vec![];

		let query = match req.uri().query() {
			Some(q) => q,
			None => return Err(ErrorKind::RequestError("no query string".to_owned()))?,
		};
		let params = form_urlencoded::parse(query.as_bytes())
			.into_owned()
			.collect::<Vec<(String, String)>>();

		for (k, id) in params {
			if k == "id" {
				for id in id.split(',') {
					commitments.push(id.to_owned());
				}
			}
		}

		let mut outputs: Vec<Output> = vec![];
		for x in commitments {
			if let Ok(output) = self.get_output(&x) {
				outputs.push(output);
			}
		}
		Ok(outputs)
	}

	fn outputs_at_height(
		&self,
		block_height: u64,
		commitments: Vec<Commitment>,
		include_proof: bool,
	) -> Result<BlockOutputs, Error> {
		let header = w(&self.chain)
			.get_header_by_height(block_height)
			.map_err(|_| ErrorKind::NotFound)?;

		// TODO - possible to compact away blocks we care about
		// in the period between accepting the block and refreshing the wallet
		let block = w(&self.chain)
			.get_block(&header.hash())
			.map_err(|_| ErrorKind::NotFound)?;
		let outputs = block
			.outputs()
			.iter()
			.filter(|output| commitments.is_empty() || commitments.contains(&output.commit))
			.map(|output| {
				OutputPrintable::from_output(output, w(&self.chain), Some(&header), include_proof)
			})
			.collect();

		Ok(BlockOutputs {
			header: BlockHeaderInfo::from_header(&header),
			outputs: outputs,
		})
	}

	// returns outputs for a specified range of blocks
	fn outputs_block_batch(&self, req: &Request<Body>) -> Result<Vec<BlockOutputs>, Error> {
		let mut commitments: Vec<Commitment> = vec![];
		let mut start_height = 1;
		let mut end_height = 1;
		let mut include_rp = false;

		let query = match req.uri().query() {
			Some(q) => q,
			None => return Err(ErrorKind::RequestError("no query string".to_owned()))?,
		};

		let params = form_urlencoded::parse(query.as_bytes()).into_owned().fold(
			HashMap::new(),
			|mut hm, (k, v)| {
				hm.entry(k).or_insert(vec![]).push(v);
				hm
			},
		);

		if let Some(ids) = params.get("id") {
			for id in ids {
				for id in id.split(',') {
					if let Ok(x) = util::from_hex(String::from(id)) {
						commitments.push(Commitment::from_vec(x));
					}
				}
			}
		}
		if let Some(heights) = params.get("start_height") {
			for height in heights {
				start_height = height
					.parse()
					.map_err(|_| ErrorKind::RequestError("invalid start_height".to_owned()))?;
			}
		}
		if let Some(heights) = params.get("end_height") {
			for height in heights {
				end_height = height
					.parse()
					.map_err(|_| ErrorKind::RequestError("invalid end_height".to_owned()))?;
			}
		}
		if let Some(_) = params.get("include_rp") {
			include_rp = true;
		}

		debug!(
			"outputs_block_batch: {}-{}, {:?}, {:?}",
			start_height, end_height, commitments, include_rp,
		);

		let mut return_vec = vec![];
		for i in (start_height..=end_height).rev() {
			if let Ok(res) = self.outputs_at_height(i, commitments.clone(), include_rp) {
				if res.outputs.len() > 0 {
					return_vec.push(res);
				}
			}
		}

		Ok(return_vec)
	}
}

impl Handler for OutputHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		let command = match req.uri().path().trim_right_matches('/').rsplit('/').next() {
			Some(c) => c,
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
		};
		match command {
			"byids" => result_to_response(self.outputs_by_ids(&req)),
			"byheight" => result_to_response(self.outputs_block_batch(&req)),
			_ => response(StatusCode::BAD_REQUEST, ""),
		}
	}
}
