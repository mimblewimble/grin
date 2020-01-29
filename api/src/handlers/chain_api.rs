// Copyright 2020 The Grin Developers
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

use super::utils::{get_output, get_output_v2, w};
use crate::chain;
use crate::core::core::hash::Hashed;
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::util;
use crate::util::secp::pedersen::Commitment;
use crate::web::*;
use failure::ResultExt;
use hyper::{Body, Request, StatusCode};
use std::sync::Weak;

/// Chain handler. Get the head details.
/// GET /v1/chain
pub struct ChainHandler {
	pub chain: Weak<chain::Chain>,
}

impl ChainHandler {
	pub fn get_tip(&self) -> Result<Tip, Error> {
		let head = w(&self.chain)?
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

impl ChainValidationHandler {
	pub fn validate_chain(&self) -> Result<(), Error> {
		w(&self.chain)?
			.validate(true)
			.map_err(|_| ErrorKind::Internal("chain error".to_owned()).into())
	}
}

impl Handler for ChainValidationHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		match w_fut!(&self.chain).validate(true) {
			Ok(_) => response(StatusCode::OK, "{}"),
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

impl ChainCompactHandler {
	pub fn compact_chain(&self) -> Result<(), Error> {
		w(&self.chain)?
			.compact()
			.map_err(|_| ErrorKind::Internal("chain error".to_owned()).into())
	}
}

impl Handler for ChainCompactHandler {
	fn post(&self, _req: Request<Body>) -> ResponseFuture {
		match w_fut!(&self.chain).compact() {
			Ok(_) => response(StatusCode::OK, "{}"),
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

	fn get_output_v2(
		&self,
		id: &str,
		include_proof: bool,
		include_merkle_proof: bool,
	) -> Result<OutputPrintable, Error> {
		let res = get_output_v2(&self.chain, id, include_proof, include_merkle_proof)?;
		Ok(res.0)
	}

	pub fn get_outputs_v2(
		&self,
		commits: Option<Vec<String>>,
		start_height: Option<u64>,
		end_height: Option<u64>,
		include_proof: Option<bool>,
		include_merkle_proof: Option<bool>,
	) -> Result<Vec<OutputPrintable>, Error> {
		let mut outputs: Vec<OutputPrintable> = vec![];
		if let Some(commits) = commits {
			// First check the commits length
			for commit in &commits {
				if commit.len() != 66 {
					return Err(ErrorKind::RequestError(format!(
						"invalid commit length for {}",
						commit
					))
					.into());
				}
			}
			for commit in commits {
				match self.get_output_v2(
					&commit,
					include_proof.unwrap_or(false),
					include_merkle_proof.unwrap_or(false),
				) {
					Ok(output) => outputs.push(output),
					// do not crash here simply do not retrieve this output
					Err(e) => error!(
						"Failure to get output for commitment {} with error {}",
						commit, e
					),
				};
			}
		}
		// cannot chain to let Some() for now  see https://github.com/rust-lang/rust/issues/53667
		if let Some(start_height) = start_height {
			if let Some(end_height) = end_height {
				let block_output_batch = self.outputs_block_batch_v2(
					start_height,
					end_height,
					include_proof.unwrap_or(false),
					include_merkle_proof.unwrap_or(false),
				)?;
				outputs = [&outputs[..], &block_output_batch[..]].concat();
			}
		}
		return Ok(outputs);
	}

	// allows traversal of utxo set
	pub fn get_unspent_outputs(
		&self,
		start_index: u64,
		end_index: Option<u64>,
		mut max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, Error> {
		//set a limit here
		if max > 10_000 {
			max = 10_000;
		}
		let chain = w(&self.chain)?;
		let outputs = chain
			.unspent_outputs_by_pmmr_index(start_index, max, end_index)
			.context(ErrorKind::NotFound)?;
		let out = OutputListing {
			last_retrieved_index: outputs.0,
			highest_index: outputs.1,
			outputs: outputs
				.2
				.iter()
				.map(|x| {
					OutputPrintable::from_output(
						x,
						chain.clone(),
						None,
						include_proof.unwrap_or(false),
						false,
					)
				})
				.collect::<Result<Vec<_>, _>>()
				.context(ErrorKind::Internal("chain error".to_owned()))?,
		};
		Ok(out)
	}

	fn outputs_by_ids(&self, req: &Request<Body>) -> Result<Vec<Output>, Error> {
		let mut commitments: Vec<String> = vec![];

		let query = must_get_query!(req);
		let params = QueryParams::from(query);
		params.process_multival_param("id", |id| commitments.push(id.to_owned()));

		let mut outputs: Vec<Output> = vec![];
		for x in commitments {
			match self.get_output(&x) {
				Ok(output) => outputs.push(output),
				Err(e) => error!(
					"Failure to get output for commitment {} with error {}",
					x, e
				),
			};
		}
		Ok(outputs)
	}

	fn outputs_at_height(
		&self,
		block_height: u64,
		commitments: Vec<Commitment>,
		include_proof: bool,
	) -> Result<BlockOutputs, Error> {
		let header = w(&self.chain)?
			.get_header_by_height(block_height)
			.map_err(|_| ErrorKind::NotFound)?;

		// TODO - possible to compact away blocks we care about
		// in the period between accepting the block and refreshing the wallet
		let chain = w(&self.chain)?;
		let block = chain
			.get_block(&header.hash())
			.map_err(|_| ErrorKind::NotFound)?;
		let outputs = block
			.outputs()
			.iter()
			.filter(|output| commitments.is_empty() || commitments.contains(&output.commit))
			.map(|output| {
				OutputPrintable::from_output(
					output,
					chain.clone(),
					Some(&header),
					include_proof,
					true,
				)
			})
			.collect::<Result<Vec<_>, _>>()
			.context(ErrorKind::Internal("cain error".to_owned()))?;

		Ok(BlockOutputs {
			header: BlockHeaderInfo::from_header(&header),
			outputs: outputs,
		})
	}

	fn outputs_at_height_v2(
		&self,
		block_height: u64,
		commitments: Vec<Commitment>,
		include_rproof: bool,
		include_merkle_proof: bool,
	) -> Result<Vec<OutputPrintable>, Error> {
		let header = w(&self.chain)?
			.get_header_by_height(block_height)
			.map_err(|_| ErrorKind::NotFound)?;

		// TODO - possible to compact away blocks we care about
		// in the period between accepting the block and refreshing the wallet
		let chain = w(&self.chain)?;
		let block = chain
			.get_block(&header.hash())
			.map_err(|_| ErrorKind::NotFound)?;
		let outputs = block
			.outputs()
			.iter()
			.filter(|output| commitments.is_empty() || commitments.contains(&output.commit))
			.map(|output| {
				OutputPrintable::from_output(
					output,
					chain.clone(),
					Some(&header),
					include_rproof,
					include_merkle_proof,
				)
			})
			.collect::<Result<Vec<_>, _>>()
			.context(ErrorKind::Internal("cain error".to_owned()))?;

		Ok(outputs)
	}

	// returns outputs for a specified range of blocks
	fn outputs_block_batch(&self, req: &Request<Body>) -> Result<Vec<BlockOutputs>, Error> {
		let mut commitments: Vec<Commitment> = vec![];

		let query = must_get_query!(req);
		let params = QueryParams::from(query);
		params.process_multival_param("id", |id| {
			if let Ok(x) = util::from_hex(String::from(id)) {
				commitments.push(Commitment::from_vec(x));
			}
		});
		let start_height = parse_param!(params, "start_height", 1);
		let end_height = parse_param!(params, "end_height", 1);
		let include_rp = params.get("include_rp").is_some();

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

	// returns outputs for a specified range of blocks
	fn outputs_block_batch_v2(
		&self,
		start_height: u64,
		end_height: u64,
		include_rproof: bool,
		include_merkle_proof: bool,
	) -> Result<Vec<OutputPrintable>, Error> {
		let commitments: Vec<Commitment> = vec![];

		debug!(
			"outputs_block_batch: {}-{}, {}, {}",
			start_height, end_height, include_rproof, include_merkle_proof,
		);

		let mut return_vec: Vec<OutputPrintable> = vec![];
		for i in (start_height..=end_height).rev() {
			if let Ok(res) = self.outputs_at_height_v2(
				i,
				commitments.clone(),
				include_rproof,
				include_merkle_proof,
			) {
				if res.len() > 0 {
					return_vec = [&return_vec[..], &res[..]].concat();
				}
			}
		}

		Ok(return_vec)
	}
}

impl Handler for OutputHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		match right_path_element!(req) {
			"byids" => result_to_response(self.outputs_by_ids(&req)),
			"byheight" => result_to_response(self.outputs_block_batch(&req)),
			_ => response(StatusCode::BAD_REQUEST, ""),
		}
	}
}

/// Kernel handler, search for a kernel by excess commitment
/// GET /v1/chain/kernels/XXX?min_height=YYY&max_height=ZZZ
/// The `min_height` and `max_height` parameters are optional
pub struct KernelHandler {
	pub chain: Weak<chain::Chain>,
}

impl KernelHandler {
	fn get_kernel(&self, req: Request<Body>) -> Result<Option<LocatedTxKernel>, Error> {
		let excess = req
			.uri()
			.path()
			.trim_end_matches('/')
			.rsplit('/')
			.next()
			.ok_or(ErrorKind::RequestError("missing excess".into()))?;
		let excess = util::from_hex(excess.to_owned())
			.map_err(|_| ErrorKind::RequestError("invalid excess hex".into()))?;
		if excess.len() != 33 {
			return Err(ErrorKind::RequestError("invalid excess length".into()).into());
		}
		let excess = Commitment::from_vec(excess);

		let chain = w(&self.chain)?;

		let mut min_height: Option<u64> = None;
		let mut max_height: Option<u64> = None;

		// Check query parameters for minimum and maximum search height
		if let Some(q) = req.uri().query() {
			let params = QueryParams::from(q);
			if let Some(h) = params.get("min_height") {
				let h = h
					.parse()
					.map_err(|_| ErrorKind::RequestError("invalid minimum height".into()))?;
				// Default is genesis
				min_height = if h == 0 { None } else { Some(h) };
			}
			if let Some(h) = params.get("max_height") {
				let h = h
					.parse()
					.map_err(|_| ErrorKind::RequestError("invalid maximum height".into()))?;
				// Default is current head
				let head_height = chain
					.head()
					.map_err(|e| ErrorKind::Internal(format!("{}", e)))?
					.height;
				max_height = if h >= head_height { None } else { Some(h) };
			}
		}

		let kernel = chain
			.get_kernel_height(&excess, min_height, max_height)
			.map_err(|e| ErrorKind::Internal(format!("{}", e)))?
			.map(|(tx_kernel, height, mmr_index)| LocatedTxKernel {
				tx_kernel,
				height,
				mmr_index,
			});
		Ok(kernel)
	}

	pub fn get_kernel_v2(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, Error> {
		let excess = util::from_hex(excess.to_owned())
			.map_err(|_| ErrorKind::RequestError("invalid excess hex".into()))?;
		if excess.len() != 33 {
			return Err(ErrorKind::RequestError("invalid excess length".into()).into());
		}
		let excess = Commitment::from_vec(excess);

		let chain = w(&self.chain)?;
		let kernel = chain
			.get_kernel_height(&excess, min_height, max_height)
			.map_err(|e| ErrorKind::Internal(format!("{}", e)))?
			.map(|(tx_kernel, height, mmr_index)| LocatedTxKernel {
				tx_kernel,
				height,
				mmr_index,
			});
		kernel.ok_or_else(|| ErrorKind::NotFound.into())
	}
}

impl Handler for KernelHandler {
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_kernel(req))
	}
}
