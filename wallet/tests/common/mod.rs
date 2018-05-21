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

//! Common functions to facilitate wallet, walletlib and transaction testing
use std::collections::hash_map::Entry;
use std::collections::HashMap;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_wallet as wallet;
extern crate time;

use chain::Chain;
use core::core::{Output, OutputFeatures, OutputIdentifier, Transaction, TxKernel};
use core::core::hash::Hashed;
use core::{consensus, global, pow};
use wallet::types::{BlockIdentifier, Error, ErrorKind, MerkleProofWrapper, OutputStatus,
                    WalletConfig, WalletData};
use wallet::{checker, BlockFees};
use keychain::Keychain;

use util::secp::pedersen;

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
		.map(|k| match get_output_local(chain, &k) {
			Err(e) => panic!(e),
			Ok(k) => k,
		})
		.collect();
	let mut api_outputs: HashMap<pedersen::Commitment, api::Output> = HashMap::new();
	for out in chain_outputs {
		api_outputs.insert(out.commit.commit(), out);
	}
	checker::apply_api_outputs(config, &wallet_outputs, &api_outputs)?;
	Ok(())
}

/// Return the spendable wallet balance from the local chain
/// (0:total, 1:amount_awaiting_confirmation, 2:confirmed but locked, 3:currently_spendable,
/// 4:locked total) TODO: Should be a wallet lib function with nicer return values
pub fn get_wallet_balances(
	config: &WalletConfig,
	keychain: &Keychain,
	height: u64,
) -> Result<(u64, u64, u64, u64, u64), Error> {
	let ret_val = WalletData::read_wallet(&config.data_file_dir, |wallet_data| {
		let mut unspent_total = 0;
		let mut unspent_but_locked_total = 0;
		let mut unconfirmed_total = 0;
		let mut locked_total = 0;
		for out in wallet_data
			.outputs
			.values()
			.filter(|out| out.root_key_id == keychain.root_key_id())
		{
			if out.status == OutputStatus::Unspent {
				unspent_total += out.value;
				if out.lock_height > height {
					unspent_but_locked_total += out.value;
				}
			}
			if out.status == OutputStatus::Unconfirmed && !out.is_coinbase {
				unconfirmed_total += out.value;
			}
			if out.status == OutputStatus::Locked {
				locked_total += out.value;
			}
		}

		Ok((
			unspent_total + unconfirmed_total,        //total
			unconfirmed_total,                        //amount_awaiting_confirmation
			unspent_but_locked_total,                 // confirmed but locked
			unspent_total - unspent_but_locked_total, // currently spendable
			locked_total,                             // locked total
		))
	});
	ret_val
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

/// Adds a block with a given reward to the chain and mines it
pub fn add_block_with_reward(chain: &Chain, txs: Vec<&Transaction>, reward: (Output, TxKernel)) {
	let prev = chain.head_header().unwrap();
	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	let mut b = core::core::Block::new(&prev, txs, difficulty.clone(), reward).unwrap();
	b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
	chain.set_txhashset_roots(&mut b, false).unwrap();
	pow::pow_size(
		&mut b.header,
		difficulty,
		global::proofsize(),
		global::sizeshift(),
	).unwrap();
	chain.process_block(b, chain::Options::MINE).unwrap();
	chain.validate(false).unwrap();
}

/// adds a reward output to a wallet, includes that reward in a block, mines the block
/// and adds it to the chain, with option transactions included.
/// Helpful for building up precise wallet balances for testing.
pub fn award_block_to_wallet(
	chain: &Chain,
	txs: Vec<&Transaction>,
	wallet: &(WalletConfig, Keychain),
) {
	let prev = chain.head_header().unwrap();
	let fee_amt = txs.iter().map(|tx| tx.fee()).sum();
	let fees = BlockFees {
		fees: fee_amt,
		key_id: None,
		height: prev.height + 1,
	};
	let coinbase_tx = wallet::receiver::receive_coinbase(&wallet.0, &wallet.1, &fees);
	let (coinbase_tx, fees) = match coinbase_tx {
		Ok(t) => ((t.0, t.1), t.2),
		Err(e) => {
			panic!("Unable to create block reward: {:?}", e);
		}
	};
	add_block_with_reward(chain, txs, coinbase_tx.clone());
	// build merkle proof and block identifier and save in wallet
	let output_id = OutputIdentifier::from_output(&coinbase_tx.0.clone());
	let m_proof = chain.get_merkle_proof(&output_id, &chain.head_header().unwrap());
	let block_id = Some(BlockIdentifier(chain.head_header().unwrap().hash()));
	let _ = WalletData::with_wallet(&wallet.0.data_file_dir, |wallet_data| {
		if let Entry::Occupied(mut output) = wallet_data
			.outputs
			.entry(fees.key_id.as_ref().unwrap().to_hex())
		{
			let output = output.get_mut();
			output.block = block_id;
			output.merkle_proof = Some(MerkleProofWrapper(m_proof.unwrap()));
		}
	});
}

/// adds many block rewards to a wallet, no transactions
pub fn award_blocks_to_wallet(
	chain: &Chain,
	wallet: &(WalletConfig, Keychain),
	num_rewards: usize,
) {
	for _ in 0..num_rewards {
		award_block_to_wallet(chain, vec![], wallet);
	}
}

/// Create a new wallet in a particular directory
pub fn create_wallet(dir: &str) -> (WalletConfig, Keychain) {
	let mut wallet_config = WalletConfig::default();
	wallet_config.data_file_dir = String::from(dir);
	let wallet_seed = wallet::WalletSeed::init_file(&wallet_config).unwrap();
	let keychain = wallet_seed
		.derive_keychain("")
		.expect("Failed to derive keychain from seed file and passphrase.");
	(wallet_config, keychain)
}
