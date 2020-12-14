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

use super::utils::{get_output, w};
use crate::chain;
use crate::core::core::hash::Hashed;
use crate::error::*;
use crate::types::*;
use crate::util;
use crate::util::secp::pedersen::Commitment;
use failure::ResultExt;
use std::sync::Weak;

/// Chain handler. Get the head details.
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

/// Chain validation handler.
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

/// Chain compaction handler. Trigger a compaction of the chain state to regain
/// storage space.
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

// Supports retrieval of multiple outputs in a single request
pub struct OutputHandler {
	pub chain: Weak<chain::Chain>,
}

impl OutputHandler {
	pub fn get_outputs(
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
				match get_output(
					&self.chain,
					&commit,
					include_proof.unwrap_or(false),
					include_merkle_proof.unwrap_or(false),
				) {
					Ok(Some((output, _))) => outputs.push(output),
					Ok(None) => {
						// Ignore outputs that are not found
					}
					Err(e) => {
						error!(
							"Failure to get output for commitment {} with error {}",
							commit, e
						);
						return Err(e);
					}
				};
			}
		}
		// cannot chain to let Some() for now  see https://github.com/rust-lang/rust/issues/53667
		if let Some(start_height) = start_height {
			if let Some(end_height) = end_height {
				let block_output_batch = self.outputs_block_batch(
					start_height,
					end_height,
					include_proof.unwrap_or(false),
					include_merkle_proof.unwrap_or(false),
				)?;
				outputs = [&outputs[..], &block_output_batch[..]].concat();
			}
		}
		Ok(outputs)
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
						&chain,
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

	fn outputs_at_height(
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
			.filter(|output| commitments.is_empty() || commitments.contains(&output.commitment()))
			.map(|output| {
				OutputPrintable::from_output(
					output,
					&chain,
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
	fn outputs_block_batch(
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
			if let Ok(res) =
				self.outputs_at_height(i, commitments.clone(), include_rproof, include_merkle_proof)
			{
				if !res.is_empty() {
					return_vec = [&return_vec[..], &res[..]].concat();
				}
			}
		}

		Ok(return_vec)
	}
}

/// Kernel handler, search for a kernel by excess commitment
/// The `min_height` and `max_height` parameters are optional
pub struct KernelHandler {
	pub chain: Weak<chain::Chain>,
}

impl KernelHandler {
	pub fn get_kernel(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, Error> {
		let excess = util::from_hex(&excess)
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

pub struct TxHashSetHandler {
	pub chain: Weak<chain::Chain>,
}

impl TxHashSetHandler {
	// allows traversal of utxo set bounded within a block range
	pub fn block_height_range_to_pmmr_indices(
		&self,
		start_block_height: u64,
		end_block_height: Option<u64>,
	) -> Result<OutputListing, Error> {
		let chain = w(&self.chain)?;
		let range = chain
			.block_height_range_to_pmmr_indices(start_block_height, end_block_height)
			.context(ErrorKind::NotFound)?;
		let out = OutputListing {
			last_retrieved_index: range.0,
			highest_index: range.1,
			outputs: vec![],
		};
		Ok(out)
	}
}
