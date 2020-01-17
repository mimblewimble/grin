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
use crate::core::core::{OutputFeatures, OutputIdentifier};
use crate::rest::*;
use crate::types::*;
use crate::util;
use crate::util::secp::pedersen::Commitment;
use failure::ResultExt;
use std::sync::{Arc, Weak};

// All handlers use `Weak` references instead of `Arc` to avoid cycles that
// can never be destroyed. These 2 functions are simple helpers to reduce the
// boilerplate of dealing with `Weak`.
pub fn w<T>(weak: &Weak<T>) -> Result<Arc<T>, Error> {
	weak.upgrade()
		.ok_or_else(|| ErrorKind::Internal("failed to upgrade weak refernce".to_owned()).into())
}

/// Retrieves an output from the chain given a commit id (a tiny bit iteratively)
pub fn get_output(
	chain: &Weak<chain::Chain>,
	id: &str,
) -> Result<(Output, OutputIdentifier), Error> {
	let c = util::from_hex(String::from(id)).context(ErrorKind::Argument(format!(
		"Not a valid commitment: {}",
		id
	)))?;
	let commit = Commitment::from_vec(c);

	// We need the features here to be able to generate the necessary hash
	// to compare against the hash in the output MMR.
	// For now we can just try both (but this probably needs to be part of the api
	// params)
	let outputs = [
		OutputIdentifier::new(OutputFeatures::Plain, &commit),
		OutputIdentifier::new(OutputFeatures::Coinbase, &commit),
	];

	let chain = w(chain)?;

	for x in outputs.iter() {
		let res = chain.is_unspent(x);
		match res {
			Ok(output_pos) => {
				return Ok((
					Output::new(&commit, output_pos.height, output_pos.position),
					x.clone(),
				));
			}
			Err(e) => {
				trace!(
					"get_output: err: {} for commit: {:?} with feature: {:?}",
					e.to_string(),
					x.commit,
					x.features
				);
			}
		}
	}
	Err(ErrorKind::NotFound)?
}

/// Retrieves an output from the chain given a commit id (a tiny bit iteratively)
pub fn get_output_v2(
	chain: &Weak<chain::Chain>,
	id: &str,
	include_proof: bool,
	include_merkle_proof: bool,
) -> Result<(OutputPrintable, OutputIdentifier), Error> {
	let c = util::from_hex(String::from(id)).context(ErrorKind::Argument(format!(
		"Not a valid commitment: {}",
		id
	)))?;
	let commit = Commitment::from_vec(c);

	// We need the features here to be able to generate the necessary hash
	// to compare against the hash in the output MMR.
	// For now we can just try both (but this probably needs to be part of the api
	// params)
	let outputs = [
		OutputIdentifier::new(OutputFeatures::Plain, &commit),
		OutputIdentifier::new(OutputFeatures::Coinbase, &commit),
	];

	let chain = w(chain)?;

	for x in outputs.iter() {
		let res = chain.is_unspent(x);
		match res {
			Ok(output_pos) => match chain.get_unspent_output_at(output_pos.position) {
				Ok(output) => {
					let mut header = None;
					if include_merkle_proof && output.is_coinbase() {
						header = chain.get_header_by_height(output_pos.height).ok();
					}
					match OutputPrintable::from_output(
						&output,
						chain.clone(),
						header.as_ref(),
						include_proof,
						include_merkle_proof,
					) {
						Ok(output_printable) => return Ok((output_printable, x.clone())),
						Err(e) => {
							trace!(
								"get_output: err: {} for commit: {:?} with feature: {:?}",
								e.to_string(),
								x.commit,
								x.features
							);
						}
					}
				}
				Err(_) => return Err(ErrorKind::NotFound)?,
			},
			Err(e) => {
				trace!(
					"get_output: err: {} for commit: {:?} with feature: {:?}",
					e.to_string(),
					x.commit,
					x.features
				);
			}
		}
	}
	Err(ErrorKind::NotFound)?
}
