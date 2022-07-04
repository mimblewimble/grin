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

use crate::chain;
use crate::chain::types::CommitPos;
use crate::core::core::OutputIdentifier;
use crate::rest::*;
use crate::types::*;
use crate::util;
use crate::util::secp::pedersen::Commitment;
use std::sync::{Arc, Weak};

// All handlers use `Weak` references instead of `Arc` to avoid cycles that
// can never be destroyed. These 2 functions are simple helpers to reduce the
// boilerplate of dealing with `Weak`.
pub fn w<T>(weak: &Weak<T>) -> Result<Arc<T>, Error> {
	weak.upgrade()
		.ok_or_else(|| Error::Internal("failed to upgrade weak reference".to_owned()))
}

/// Internal function to retrieves an output by a given commitment
fn get_unspent(
	chain: &Arc<chain::Chain>,
	id: &str,
) -> Result<Option<(OutputIdentifier, CommitPos)>, Error> {
	let c = util::from_hex(id)
		.map_err(|_| Error::Argument(format!("Not a valid commitment: {}", id)))?;
	let commit = Commitment::from_vec(c);
	let res = chain.get_unspent(commit)?;
	Ok(res)
}

/// Retrieves an output from the chain given a commitment.
pub fn get_output(
	chain: &Weak<chain::Chain>,
	id: &str,
) -> Result<Option<(Output, OutputIdentifier)>, Error> {
	let chain = w(chain)?;
	let (out, pos) = match get_unspent(&chain, id)? {
		Some(x) => x,
		None => return Ok(None),
	};

	Ok(Some((
		Output::new(&out.commitment(), pos.height, pos.pos),
		out,
	)))
}

/// Retrieves an output from the chain given a commit id (a tiny bit iteratively)
pub fn get_output_v2(
	chain: &Weak<chain::Chain>,
	id: &str,
	include_proof: bool,
	include_merkle_proof: bool,
) -> Result<Option<(OutputPrintable, OutputIdentifier)>, Error> {
	let chain = w(chain)?;
	let (out, pos) = match get_unspent(&chain, id)? {
		Some(x) => x,
		None => return Ok(None),
	};

	let output = chain.get_unspent_output_at(pos.pos - 1)?;
	let header = if include_merkle_proof && output.is_coinbase() {
		chain.get_header_by_height(pos.height).ok()
	} else {
		None
	};

	let output_printable = OutputPrintable::from_output(
		&output,
		&chain,
		header.as_ref(),
		include_proof,
		include_merkle_proof,
	)?;

	Ok(Some((output_printable, out)))
}
