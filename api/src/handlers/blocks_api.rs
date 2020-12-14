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
use crate::chain;
use crate::core::core::hash::Hash;
use crate::core::core::hash::Hashed;
use crate::error::*;
use crate::handlers::utils::{get_output, w};
use crate::types::*;
use failure::ResultExt;
use std::sync::Weak;

/// Gets block headers given either a hash or height or an output commit.
pub struct HeaderHandler {
	pub chain: Weak<chain::Chain>,
}

impl HeaderHandler {
	pub fn get_header(&self, h: &Hash) -> Result<BlockHeaderPrintable, Error> {
		let chain = w(&self.chain)?;
		let header = chain.get_block_header(h).context(ErrorKind::NotFound)?;
		Ok(BlockHeaderPrintable::from_header(&header))
	}

	// Try to get hash from height, hash or output commit
	pub fn parse_inputs(
		&self,
		height: Option<u64>,
		hash: Option<Hash>,
		commit: Option<String>,
	) -> Result<Hash, Error> {
		if let Some(height) = height {
			match w(&self.chain)?.get_header_by_height(height) {
				Ok(header) => return Ok(header.hash()),
				Err(_) => return Err(ErrorKind::NotFound.into()),
			}
		}
		if let Some(hash) = hash {
			return Ok(hash);
		}
		if let Some(commit) = commit {
			let oid = match get_output(&self.chain, &commit, false, false)? {
				Some((_, o)) => o,
				None => return Err(ErrorKind::NotFound.into()),
			};
			match w(&self.chain)?.get_header_for_output(oid.commitment()) {
				Ok(header) => return Ok(header.hash()),
				Err(_) => return Err(ErrorKind::NotFound.into()),
			}
		}
		Err(ErrorKind::Argument("not a valid hash, height or output commit".to_owned()).into())
	}
}

/// Gets block details given either a hash or an unspent commit
///
/// Optionally return results as "compact blocks" by passing "?compact" query
///
/// Optionally turn off the Merkle proof extraction by passing "?no_merkle_proof" query
pub struct BlockHandler {
	pub chain: Weak<chain::Chain>,
}

impl BlockHandler {
	pub fn get_block(
		&self,
		h: &Hash,
		include_proof: bool,
		include_merkle_proof: bool,
	) -> Result<BlockPrintable, Error> {
		let chain = w(&self.chain)?;
		let block = chain.get_block(h).context(ErrorKind::NotFound)?;
		BlockPrintable::from_block(&block, &chain, include_proof, include_merkle_proof)
			.map_err(|_| ErrorKind::Internal("chain error".to_owned()).into())
	}

	// Try to get hash from height, hash or output commit
	pub fn parse_inputs(
		&self,
		height: Option<u64>,
		hash: Option<Hash>,
		commit: Option<String>,
	) -> Result<Hash, Error> {
		if let Some(height) = height {
			match w(&self.chain)?.get_header_by_height(height) {
				Ok(header) => return Ok(header.hash()),
				Err(_) => return Err(ErrorKind::NotFound.into()),
			}
		}
		if let Some(hash) = hash {
			return Ok(hash);
		}
		if let Some(commit) = commit {
			let oid = match get_output(&self.chain, &commit, false, false)? {
				Some((_, o)) => o,
				None => return Err(ErrorKind::NotFound.into()),
			};
			match w(&self.chain)?.get_header_for_output(oid.commitment()) {
				Ok(header) => return Ok(header.hash()),
				Err(_) => return Err(ErrorKind::NotFound.into()),
			}
		}
		Err(ErrorKind::Argument("not a valid hash, height or output commit".to_owned()).into())
	}
}
