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
use failure::{Fail, ResultExt};
use keychain::{Identifier, Keychain};
use util::{to_hex, LOGGER};
use util::secp::pedersen;
use api;
use core::global;
use core::core::{Output, SwitchCommitHash};
use core::core::transaction::OutputFeatures;
use types::{Error, ErrorKind, OutputData, OutputStatus, WalletConfig, WalletData};
use byteorder::{BigEndian, ByteOrder};

pub fn get_chain_height(config: &WalletConfig) -> Result<u64, Error> {
	let url = format!("{}/v1/chain", config.check_node_api_http_addr);

	match api::client::get::<api::Tip>(url.as_str()) {
		Ok(tip) => Ok(tip.height),
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(
				LOGGER,
				"get_chain_height: Restore failed... unable to contact API {}. Error: {}",
				config.check_node_api_http_addr,
				e
			);
			Err(e.context(ErrorKind::Node).into())
		}
	}
}

fn output_with_range_proof(
	config: &WalletConfig,
	commit_id: &str,
	height: u64,
) -> Result<api::OutputPrintable, Error> {
	let url = format!(
		"{}/v1/chain/utxos/byheight?start_height={}&end_height={}&id={}&include_rp",
		config.check_node_api_http_addr, height, height, commit_id,
	);

	match api::client::get::<Vec<api::BlockOutputs>>(url.as_str()) {
		Ok(block_outputs) => {
			if let Some(block_output) = block_outputs.first() {
				if let Some(output) = block_output.outputs.first() {
					Ok(output.clone())
				} else {
					Err(ErrorKind::Node)?
				}
			} else {
				Err(ErrorKind::Node)?
			}
		}
		Err(e) => {
			// if we got anything other than 200 back from server, don't attempt to refresh
			// the wallet
			// data after
			Err(e.context(ErrorKind::Node))?
		}
	}
}

fn retrieve_amount_and_coinbase_status(
	config: &WalletConfig,
	keychain: &Keychain,
	key_id: Identifier,
	commit_id: &str,
	height: u64,
) -> Result<(u64, bool), Error> {
	let output = output_with_range_proof(config, commit_id, height)?;

	let core_output = Output {
		features: match output.output_type {
			api::OutputType::Coinbase => OutputFeatures::COINBASE_OUTPUT,
			api::OutputType::Transaction => OutputFeatures::DEFAULT_OUTPUT,
		},
		proof: output
			.range_proof()
			.context(ErrorKind::GenericError("range proof error"))?,
		switch_commit_hash: output
			.switch_commit_hash()
			.context(ErrorKind::GenericError("switch commit hash error"))?,
		commit: output
			.commit()
			.context(ErrorKind::GenericError("commit error"))?,
	};

	if let Some(amount) = core_output.recover_value(keychain, &key_id) {
		let is_coinbase = match output.output_type {
			api::OutputType::Coinbase => true,
			api::OutputType::Transaction => false,
		};
		Ok((amount, is_coinbase))
	} else {
		Err(ErrorKind::GenericError("cannot recover value"))?
	}
}

pub fn utxos_batch_block(
	config: &WalletConfig,
	start_height: u64,
	end_height: u64,
) -> Result<Vec<api::BlockOutputs>, Error> {
	let query_param = format!("start_height={}&end_height={}", start_height, end_height);

	let url = format!(
		"{}/v1/chain/utxos/byheight?{}",
		config.check_node_api_http_addr, query_param,
	);

	match api::client::get::<Vec<api::BlockOutputs>>(url.as_str()) {
		Ok(outputs) => Ok(outputs),
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(
				LOGGER,
				"utxos_batch_block: Restore failed... unable to contact API {}. Error: {}",
				config.check_node_api_http_addr,
				e
			);
			Err(e.context(ErrorKind::Node))?
		}
	}
}

// TODO - wrap the many return values in a struct
fn find_utxos_with_key(
	config: &WalletConfig,
	keychain: &Keychain,
	switch_commit_cache: &Vec<pedersen::Commitment>,
	block_outputs: api::BlockOutputs,
	key_iterations: &mut usize,
	padding: &mut usize,
) -> Vec<(pedersen::Commitment, Identifier, u32, u64, u64, u64, bool)> {
	let mut wallet_outputs: Vec<(pedersen::Commitment, Identifier, u32, u64, u64, u64, bool)> =
		Vec::new();

	info!(
		LOGGER,
		"Scanning block {}, {} outputs, over {} key derivations",
		block_outputs.header.height,
		block_outputs.outputs.len(),
		*key_iterations,
	);

	for output in block_outputs.outputs.iter().filter(|x| !x.spent) {
		for i in 1..*key_iterations {
			let key_id = &keychain.derive_key_id(i as u32).unwrap();

			if let Ok(x) = output.switch_commit_hash() {
				let expected_hash = SwitchCommitHash::from_switch_commit(
					switch_commit_cache[i as usize],
					&keychain,
					&key_id,
				);

				if x == expected_hash {
					info!(LOGGER, "Output found: {:?}, key_index: {:?}", output, i,);

					// add it to result set here
					let commit_id = output.commit.0;

					let res = retrieve_amount_and_coinbase_status(
						config,
						keychain,
						key_id.clone(),
						&to_hex(output.commit.0.to_vec()),
						block_outputs.header.height,
					);

					if let Ok((amount, is_coinbase)) = res {
						info!(LOGGER, "Amount: {}", amount);

						let commit = keychain
							.commit_with_key_index(BigEndian::read_u64(&commit_id), i as u32)
							.expect("commit with key index");

						let height = block_outputs.header.height;
						let lock_height = if is_coinbase {
							height + global::coinbase_maturity()
						} else {
							0
						};

						wallet_outputs.push((
							commit,
							key_id.clone(),
							i as u32,
							amount,
							height,
							lock_height,
							is_coinbase,
						));

						// probably don't have to look for indexes greater than this now
						*key_iterations = i + *padding;
						if *key_iterations > switch_commit_cache.len() {
							*key_iterations = switch_commit_cache.len();
						}
						info!(LOGGER, "Setting max key index to: {}", *key_iterations);
						break;
					} else {
						info!(
							LOGGER,
							"Unable to retrieve the amount (needs investigating) {:?}", res,
						);
					}
				}
			}
		}
	}
	debug!(
		LOGGER,
		"Found {} wallet_outputs for block {}",
		wallet_outputs.len(),
		block_outputs.header.height,
	);

	wallet_outputs
}

pub fn restore(
	config: &WalletConfig,
	keychain: &Keychain,
	key_derivations: u32,
) -> Result<(), Error> {
	// Don't proceed if wallet.dat has anything in it
	let is_empty = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		Ok(wallet_data.outputs.len() == 0)
	}).context(ErrorKind::WalletData("could not read wallet"))?;
	if !is_empty {
		error!(
			LOGGER,
			"Not restoring. Please back up and remove existing wallet.dat first."
		);
		return Ok(());
	}

	// Get height of chain from node (we'll check again when done)
	let chain_height = get_chain_height(config)?;
	info!(
		LOGGER,
		"Starting restore: Chain height is {}.", chain_height
	);

	let mut switch_commit_cache: Vec<pedersen::Commitment> = vec![];
	info!(
		LOGGER,
		"Building key derivation cache ({}) ...", key_derivations,
	);
	for i in 0..key_derivations {
		let switch_commit = keychain.switch_commit_from_index(i as u32).unwrap();
		switch_commit_cache.push(switch_commit);
	}
	debug!(LOGGER, "... done");

	let batch_size = 100;
	// this will start here, then lower as outputs are found, moving backwards on
	// the chain
	let mut key_iterations = key_derivations as usize;
	// set to a percentage of the key_derivation value
	let mut padding = (key_iterations as f64 * 0.25) as usize;
	let mut h = chain_height;
	while {
		let end_batch = h;
		if h >= batch_size {
			h -= batch_size;
		} else {
			h = 0;
		}
		let mut blocks = utxos_batch_block(config, h + 1, end_batch)?;
		blocks.reverse();

		let _ = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
			for block in blocks {
				let result_vec = find_utxos_with_key(
					config,
					keychain,
					&switch_commit_cache,
					block,
					&mut key_iterations,
					&mut padding,
				);
				if result_vec.len() > 0 {
					for output in result_vec.clone() {
						let root_key_id = keychain.root_key_id();
						// Just plonk it in for now, and refresh actual values via wallet info
						// command later
						wallet_data.add_output(OutputData {
							root_key_id: root_key_id.clone(),
							key_id: output.1.clone(),
							n_child: output.2,
							value: output.3,
							status: OutputStatus::Unconfirmed,
							height: output.4,
							lock_height: output.5,
							is_coinbase: output.6,
							block: None,
							merkle_proof: None,
						});
					}
				}
			}
		});
		h > 0
	} {}
	Ok(())
}
