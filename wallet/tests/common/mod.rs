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

//! Common functions to facilitate wallet testing
use std::collections::hash_map::Entry;
use std::collections::HashMap;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_wallet as wallet;

use wallet::types::*;
use keychain::{Identifier, Keychain};
use util::secp::pedersen;
use util;
use util::LOGGER;

use wallet::checker;

use core::core::transaction::{OutputFeatures, OutputIdentifier};

/// Mostly for testing, refreshes output state against a local chain instance instead of
/// via an http API call
pub fn refresh_output_state_local(
	config: &WalletConfig,
	keychain: &Keychain,
	chain: &chain::Chain,
) -> Result<(), Error> {
	let wallet_outputs = checker::map_wallet_outputs(config, keychain)?;
	let chain_outputs: Vec<api::Output> = wallet_outputs
		.keys()
		.map(|k| get_output_local(chain, &k).unwrap())
		.collect();
	let mut api_outputs: HashMap<pedersen::Commitment, api::Output> = HashMap::new();
	for out in chain_outputs {
		api_outputs.insert(out.commit.commit(), out);
	}
	checker::apply_api_outputs(config, &wallet_outputs, &api_outputs)?;
	Ok(())
}

/// Get an output from the chain locally and present it back as an API output
fn get_output_local(
	chain: &chain::Chain,
	commit: &pedersen::Commitment,
) -> Result<api::Output, Error> {
	let outputs = [
		OutputIdentifier::new(OutputFeatures::DEFAULT_OUTPUT, commit),
		OutputIdentifier::new(OutputFeatures::COINBASE_OUTPUT, commit),
	];

	for x in outputs.iter() {
		if let Ok(_) = chain.is_unspent(&x) {
			return Ok(api::Output::new(&commit));
		}
	}
	Err(ErrorKind::Transaction)?
}
