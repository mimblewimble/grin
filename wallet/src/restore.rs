// Copyright 2017 The Grin Developers
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

use keychain::{Keychain, Identifier};
use util::{LOGGER, from_hex};
use util::secp::pedersen;
use api;
use core::core::{Output,SwitchCommitHash};
use core::core::transaction::{COINBASE_OUTPUT, DEFAULT_OUTPUT, SWITCH_COMMIT_HASH_SIZE};
use types::{WalletConfig, WalletData, OutputData, OutputStatus, Error};
use byteorder::{BigEndian, ByteOrder};

pub fn get_chain_height(config: &WalletConfig)->
	Result<u64, Error>{
	let url = format!(
		"{}/v1/chain",
		config.check_node_api_http_addr
	);

	match api::client::get::<api::Tip>(url.as_str()) {
		Ok(tip) => {
			Ok(tip.height)
		},
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(LOGGER, "Restore failed... unable to contact node: {}", e);
			Err(Error::Node(e))
		}
	}
}

fn output_with_range_proof(config:&WalletConfig, commit_id: &str) ->
	Result<api::Output, Error>{

	let url = format!(
		"{}/v1/chain/utxos/byids?id={}&include_rp&include_switch",
		config.check_node_api_http_addr,
		commit_id,
	);

	match api::client::get::<Vec<api::Output>>(url.as_str()) {
		Ok(outputs) => {
			Ok(outputs[0].clone())
		},
		Err(e) => {
			// if we got anything other than 200 back from server, don't attempt to refresh the wallet
			// data after
			Err(Error::Node(e))
		}
	}
}

fn retrieve_amount_and_coinbase_status(config:&WalletConfig, keychain: &Keychain,
	key_id: Identifier, commit_id: &str) -> (u64, bool) {
	let output = output_with_range_proof(config, commit_id).unwrap();
	let core_output = Output {
		features : match output.output_type {
			api::OutputType::Coinbase => COINBASE_OUTPUT,
			api::OutputType::Transaction => DEFAULT_OUTPUT,
		},
		proof: output.proof.unwrap(),
		switch_commit_hash: output.switch_commit_hash.unwrap(),
		commit: output.commit,
	};
	let amount=core_output.recover_value(keychain, &key_id).unwrap();
	let is_coinbase = match output.output_type {
		api::OutputType::Coinbase => true,
		api::OutputType::Transaction => false,
	};
	(amount, is_coinbase)
}

pub fn utxos_batch_block(config: &WalletConfig, start_height: u64, end_height:u64)->
	Result<Vec<api::BlockOutputs>, Error>{
	// build the necessary query param -
	// ?height=x
	let query_param= format!("start_height={}&end_height={}", start_height, end_height);

	let url = format!(
		"{}/v1/chain/utxos/atheight?{}",
		config.check_node_api_http_addr,
		query_param,
	);

	match api::client::get::<Vec<api::BlockOutputs>>(url.as_str()) {
		Ok(outputs) => Ok(outputs),
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(LOGGER, "Restore failed... unable to contact node: {}", e);
			Err(Error::Node(e))
		}
	}
}

fn find_utxos_with_key(config:&WalletConfig, keychain: &Keychain, 
	switch_commit_cache : &Vec<[u8;SWITCH_COMMIT_HASH_SIZE]>,
	block_outputs:api::BlockOutputs, key_iterations: &mut usize, padding: &mut usize) 
	-> Vec<(pedersen::Commitment, Identifier, u32, u64, u64, bool) > {
	//let key_id = keychain.clone().root_key_id();
	let mut wallet_outputs: Vec<(pedersen::Commitment, Identifier, u32, u64, u64, bool)> = Vec::new();

	info!(LOGGER, "Scanning block {} over {} key derivation possibilities.", block_outputs.header.height, *key_iterations);
	for output in block_outputs.outputs {
		for i in 0..*key_iterations {
			if switch_commit_cache[i as usize]==output.switch_commit_hash {
				info!(LOGGER, "Output found: {:?}, key_index: {:?}", output.switch_commit_hash,i);
				//add it to result set here
				let commit_id = from_hex(output.commit.clone()).unwrap();
				let key_id = keychain.derive_key_id(i as u32).unwrap();
				let (amount, is_coinbase) = retrieve_amount_and_coinbase_status(config,
					keychain, key_id.clone(), &output.commit);
				info!(LOGGER, "Amount: {}", amount);
				let commit = keychain.commit_with_key_index(BigEndian::read_u64(&commit_id), i as u32).unwrap();
				wallet_outputs.push((commit, key_id.clone(), i as u32, amount, output.height, is_coinbase));
				//probably don't have to look for indexes greater than this now
				*key_iterations=i+*padding;
				if *key_iterations > switch_commit_cache.len() {
					*key_iterations = switch_commit_cache.len();
				}
				info!(LOGGER, "Setting max key index to: {}", *key_iterations);
				break;
			}
		}
	}
	wallet_outputs
}

pub fn restore(config: &WalletConfig, keychain: &Keychain, key_derivations:u32) ->
	Result<(), Error>{
	// Don't proceed if wallet.dat has anything in it
	let is_empty = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		wallet_data.outputs.len() ==  0
	})?;
	if !is_empty {
		error!(LOGGER, "Not restoring. Please back up and remove existing wallet.dat first.");
		return Ok(())
	}

// Get height of chain from node (we'll check again when done)
	let chain_height = get_chain_height(config)?;
	info!(LOGGER, "Starting restore: Chain height is {}.", chain_height);

	let mut switch_commit_cache : Vec<[u8;SWITCH_COMMIT_HASH_SIZE]> = vec![];
	info!(LOGGER, "Building key derivation cache to index {}.", key_derivations);
	for i in 0..key_derivations {
		let switch_commit = keychain.switch_commit_from_index(i as u32).unwrap();
		let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
		switch_commit_cache.push(switch_commit_hash.hash);
	}

	let batch_size=100;
	//this will start here, then lower as outputs are found, moving backwards on the chain
	let mut key_iterations=key_derivations as usize;
	//set to a percentage of the key_derivation value
	let mut padding = (key_iterations as f64 *0.25) as usize;
	let mut h = chain_height;
	while {
		let end_batch=h;
		if h >= batch_size {
			h-=batch_size;
		} else {
			h=0;
		}
		let mut blocks = utxos_batch_block(config, h+1, end_batch)?;
		blocks.reverse();
		for block in blocks {
			let result_vec=find_utxos_with_key(config, keychain, &switch_commit_cache,
				block, &mut key_iterations, &mut padding);
			if result_vec.len() > 0 {
				for output in result_vec.clone() {
					let _ = WalletData::with_wallet(&config.data_file_dir, |wallet_data| {
						let root_key_id = keychain.root_key_id();
						//Just plonk it in for now, and refresh actual values via wallet info command later 
						wallet_data.add_output(OutputData {
							root_key_id: root_key_id.clone(),
							key_id: output.1.clone(),
							n_child: output.2,
							value: output.3,
							status: OutputStatus::Unconfirmed,
							height: output.4,
							lock_height: 0,
							is_coinbase: output.5,
						});
					});
				}
			}
		}
		h > 0
	}{} 
	Ok(())
}
