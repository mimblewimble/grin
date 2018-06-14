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
//! Functions to restore a wallet's outputs from just the master seed

/// TODO: Remove api
use api;
use byteorder::{BigEndian, ByteOrder};
use core::core::transaction::ProofMessageElements;
use core::global;
use error::{Error, ErrorKind};
use failure::{Fail, ResultExt};
use keychain::{Identifier, Keychain};
use libtx::proof;
use libwallet::types::*;
use util::secp::pedersen;
use util::{self, LOGGER};

fn get_merkle_proof_for_commit(node_addr: &str, commit: &str) -> Result<MerkleProofWrapper, Error> {
	let url = format!("{}/v1/txhashset/merkleproof?id={}", node_addr, commit);

	match api::client::get::<api::OutputPrintable>(url.as_str()) {
		Ok(output) => Ok(MerkleProofWrapper(output.merkle_proof.unwrap())),
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(
				LOGGER,
				"get_merkle_proof_for_pos: Restore failed... unable to create merkle proof for commit {}. Error: {}",
				commit,
				e
			);
			Err(e.context(ErrorKind::Node).into())
		}
	}
}
fn coinbase_status(output: &api::OutputPrintable) -> bool {
	match output.output_type {
		api::OutputType::Coinbase => true,
		api::OutputType::Transaction => false,
	}
}

fn outputs_batch<T, K>(wallet: &T, start_height: u64, max: u64) -> Result<api::OutputListing, Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let query_param = format!("start_index={}&max={}", start_height, max);

	let url = format!("{}/v1/txhashset/outputs?{}", wallet.node_url(), query_param,);

	match api::client::get::<api::OutputListing>(url.as_str()) {
		Ok(o) => Ok(o),
		Err(e) => {
			// if we got anything other than 200 back from server, bye
			error!(
				LOGGER,
				"outputs_batch: Restore failed... unable to contact API {}. Error: {}",
				wallet.node_url(),
				e
			);
			Err(e.context(ErrorKind::Node))?
		}
	}
}

// TODO - wrap the many return values in a struct
fn find_outputs_with_key<T, K>(
	wallet: &mut T,
	outputs: Vec<api::OutputPrintable>,
	found_key_index: &mut Vec<u32>,
) -> Vec<(
	pedersen::Commitment,
	Identifier,
	u32,
	u64,
	u64,
	u64,
	bool,
	Option<MerkleProofWrapper>,
)>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	let mut wallet_outputs: Vec<(
		pedersen::Commitment,
		Identifier,
		u32,
		u64,
		u64,
		u64,
		bool,
		Option<MerkleProofWrapper>,
	)> = Vec::new();

	let max_derivations = 1_000_000;

	info!(LOGGER, "Scanning {} outputs", outputs.len(),);
	let current_chain_height = wallet.get_chain_height(wallet.node_url()).unwrap();

	// skey doesn't matter in this case
	let skey = wallet.keychain().derive_key_id(1).unwrap();
	for output in outputs.iter().filter(|x| !x.spent) {
		// attempt to unwind message from the RP and get a value.. note
		// this will only return okay if the value is included in the
		// message 3 times, indicating a strong match. Also, sec_key provided
		// to unwind in this case will be meaningless. With only the nonce known
		// only the first 32 bytes of the recovered message will be accurate
		let info = proof::rewind(
			wallet.keychain(),
			&skey,
			output.commit,
			None,
			output.range_proof().unwrap(),
		).unwrap();
		let message = ProofMessageElements::from_proof_message(&info.message).unwrap();
		let value = message.value();
		if value.is_err() {
			continue;
		}
		// we have a match, now check through our key iterations to find a partial match
		let mut found = false;

		let mut start_index = 1;

		// TODO: This assumption only holds with current wallet software assuming
		// wallet doesn't go back and re-use gaps in its key index, ie. every
		// new key index produced is always greater than the previous max key index
		if let Some(m) = found_key_index.iter().max() {
			start_index = *m as usize + 1;
		}

		for i in start_index..max_derivations {
			// much faster than calling EC functions for each found key
			// Shouldn't be needed if assumption about wallet key 'gaps' above
			// holds.. otherwise this is a good optimization.. perhaps 
			// provide a command line switch
			/*if found_key_index.contains(&(i as u32)) {
				continue;
			}*/
			let key_id = &wallet.keychain().derive_key_id(i as u32).unwrap();
			if !message.compare_bf_first_8(key_id) {
				continue;
			}
			found = true;
			// we have a partial match, let's just confirm
			let info = proof::rewind(
				wallet.keychain(),
				key_id,
				output.commit,
				None,
				output.range_proof().unwrap(),
			).unwrap();
			let message = ProofMessageElements::from_proof_message(&info.message).unwrap();
			let value = message.value();
			if value.is_err() || !message.zeroes_correct() {
				continue;
			}
			let value = value.unwrap();
			info!(
				LOGGER,
				"Output found: {:?}, key_index: {:?}", output.commit, i,
			);
			found_key_index.push(i as u32);
			// add it to result set here
			let commit_id = output.commit.0;

			let is_coinbase = coinbase_status(output);

			info!(LOGGER, "Amount: {}", value);

			let commit = wallet
				.keychain()
				.commit_with_key_index(BigEndian::read_u64(&commit_id), i as u32)
				.expect("commit with key index");

			let mut merkle_proof = None;
			let commit_str = util::to_hex(output.commit.as_ref().to_vec());

			if is_coinbase {
				merkle_proof =
					Some(get_merkle_proof_for_commit(wallet.node_url(), &commit_str).unwrap());
			}

			let height = current_chain_height;
			let lock_height = if is_coinbase {
				height + global::coinbase_maturity()
			} else {
				height
			};

			wallet_outputs.push((
				commit,
				key_id.clone(),
				i as u32,
				value,
				height,
				lock_height,
				is_coinbase,
				merkle_proof,
			));

			break;
		}
		if !found {
			warn!(
				LOGGER,
				"Very probable matching output found with amount: {} \
				 but didn't match key child key up to {}",
				message.value().unwrap(),
				max_derivations,
			);
		}
	}
	debug!(LOGGER, "Found {} wallet_outputs", wallet_outputs.len(),);

	wallet_outputs
}

/// Restore a wallet
pub fn restore<T, K>(wallet: &mut T) -> Result<(), Error>
where
	T: WalletBackend<K> + WalletClient,
	K: Keychain,
{
	// Don't proceed if wallet.dat has anything in it
	let is_empty = wallet
		.read_wallet(|wallet_data| Ok(wallet_data.outputs().len() == 0))
		.context(ErrorKind::FileWallet("could not read wallet"))?;
	if !is_empty {
		error!(
			LOGGER,
			"Not restoring. Please back up and remove existing wallet.dat first."
		);
		return Ok(());
	}

	info!(LOGGER, "Starting restore.");

	let batch_size = 1000;
	let mut start_index = 1;
	// Keep a set of keys we've already claimed (cause it's far faster than
	// deriving a key for each one)
	let mut found_key_index: Vec<u32> = vec![];
	// this will start here, then lower as outputs are found, moving backwards on
	// the chain
	loop {
		let output_listing = outputs_batch(wallet, start_index, batch_size)?;
		info!(
			LOGGER,
			"Retrieved {} outputs, up to index {}. (Highest index: {})",
			output_listing.outputs.len(),
			output_listing.last_retrieved_index,
			output_listing.highest_index
		);

		let _ = wallet.with_wallet(|wallet_data| {
			let keychain = wallet_data.keychain().clone();
			let result_vec = find_outputs_with_key(
				wallet_data,
				output_listing.outputs.clone(),
				&mut found_key_index,
			);
			if result_vec.len() > 0 {
				for output in result_vec.clone() {
					let root_key_id = keychain.root_key_id();
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
						merkle_proof: output.7,
					});
				}
			}
		});
		if output_listing.highest_index == output_listing.last_retrieved_index {
			break;
		}
		start_index = output_listing.last_retrieved_index + 1;
	}
	Ok(())
}
