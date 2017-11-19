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
use util::{LOGGER, to_hex, from_hex};
use util::secp::pedersen;
use api;
use core::core::{Output,SwitchCommitHash};
use core::core::transaction::{COINBASE_OUTPUT, DEFAULT_OUTPUT};
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

fn retrieve_amount(config:&WalletConfig, keychain: &Keychain, key_id: Identifier, commit_id: &str) -> u64 {
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
	amount
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
	block_outputs:api::BlockOutputs, key_iterations: u64) 
	-> Vec<(pedersen::Commitment, Identifier, u32, u64, u64) > {
	//let key_id = keychain.clone().root_key_id();
	let mut wallet_outputs: Vec<(pedersen::Commitment, Identifier, u32, u64, u64)> = Vec::new();

	info!(LOGGER, "Scanning block {}", block_outputs.header.height);
	for output in block_outputs.outputs {
		for i in 1..key_iterations+1 {
			let key_id = keychain.derive_key_id(i as u32).unwrap();
			let switch_commit = keychain.switch_commit(&key_id).unwrap();
			let switch_commit_hash = SwitchCommitHash::from_switch_commit(switch_commit);
			//println!("switch commit hash: {:?}", switch_commit_hash);
			let compare_string=to_hex(switch_commit_hash.hash.to_vec());
			if compare_string==output.switch_commit_hash {
				info!(LOGGER, "Output found: {:?}", output.switch_commit_hash);
				//add it to result set here
				let commit_id = from_hex(output.commit.clone()).unwrap();
				let amount = retrieve_amount(config, keychain, key_id.clone(), &output.commit);
				info!(LOGGER, "Amount: {}", amount);
				let commit = keychain.commit_with_key_index(BigEndian::read_u64(&commit_id), i as u32).unwrap();
				wallet_outputs.push((commit, key_id.clone(), i as u32, amount, output.height));
				break;
			}
		}
	}
	wallet_outputs
}

pub fn restore(config: &WalletConfig, keychain: &Keychain) ->
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
	debug!(LOGGER, "Restore: Chain height is {}", chain_height);

	let mut batch_size=100;
	let key_iterations=1000;
	for h in 1..chain_height+1 {
		if h % batch_size != 0 && h!=chain_height{
			continue;
		}
		if h==chain_height {
			batch_size=h%batch_size;
		}
		let blocks = utxos_batch_block(config, h-batch_size+1, h)?;
		for block in blocks{
			let result_vec=find_utxos_with_key(config, keychain, block, key_iterations);
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
							is_coinbase: false,
						});
					});
				}
			}
		}
	}
	Ok(())
}
