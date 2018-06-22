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
use std::collections::HashMap;
use std::collections::hash_map::Entry;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_wallet as wallet;
extern crate time;

use chain::Chain;
use core::core::hash::Hashed;
use core::core::{Output, OutputFeatures, OutputIdentifier, Transaction, TxKernel};
use core::{consensus, global, pow};
use keychain::ExtKeychain;
use wallet::WalletConfig;
use wallet::file_wallet::FileWallet;
use wallet::libwallet::internal::updater;
use wallet::libwallet::types::{BlockFees, BlockIdentifier, MerkleProofWrapper, OutputStatus,
                               WalletBackend};
use wallet::libwallet::{Error, ErrorKind};

use util;
use util::secp::pedersen;

/// Mostly for testing, refreshes output state against a local chain instance
/// instead of via an http API call
pub fn refresh_output_state_local<T, K>(wallet: &mut T, chain: &chain::Chain) -> Result<(), Error>
where
	T: WalletBackend<K>,
	K: keychain::Keychain,
{
	let wallet_outputs = updater::map_wallet_outputs(wallet)?;
	let chain_outputs: Vec<Option<api::Output>> = wallet_outputs
		.keys()
		.map(|k| match get_output_local(chain, &k) {
			Err(_) => None,
			Ok(k) => Some(k),
		})
		.collect();
	let mut api_outputs: HashMap<pedersen::Commitment, String> = HashMap::new();
	for out in chain_outputs {
		match out {
			Some(o) => {
				api_outputs.insert(o.commit.commit(), util::to_hex(o.commit.to_vec()));
			}
			None => {}
		}
	}
	let height = chain.head().unwrap().height;
	updater::apply_api_outputs(wallet, &wallet_outputs, &api_outputs, height)?;
	Ok(())
}

/// Return the spendable wallet balance from the local chain
/// (0:total, 1:amount_awaiting_confirmation, 2:confirmed but locked,
/// 3:currently_spendable, 4:locked total) TODO: Should be a wallet lib
/// function with nicer return values
pub fn get_wallet_balances<T, K>(
	wallet: &mut T,
	height: u64,
) -> Result<(u64, u64, u64, u64, u64), Error>
where
	T: WalletBackend<K>,
	K: keychain::Keychain,
{
	let mut unspent_total = 0;
	let mut unspent_but_locked_total = 0;
	let mut unconfirmed_total = 0;
	let mut locked_total = 0;
	let keychain = wallet.keychain().clone();
	for out in wallet
		.iter()
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
	Err(ErrorKind::GenericError(
		"Can't get output from local instance of chain",
	))?
}

/// Adds a block with a given reward to the chain and mines it
pub fn add_block_with_reward(chain: &Chain, txs: Vec<&Transaction>, reward: (Output, TxKernel)) {
	let prev = chain.head_header().unwrap();
	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	let mut b = core::core::Block::new(
		&prev,
		txs.into_iter().cloned().collect(),
		difficulty.clone(),
		reward,
	).unwrap();
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

/// adds a reward output to a wallet, includes that reward in a block, mines
/// the block and adds it to the chain, with option transactions included.
/// Helpful for building up precise wallet balances for testing.
pub fn award_block_to_wallet<T, K>(chain: &Chain, txs: Vec<&Transaction>, wallet: &mut T)
where
	T: WalletBackend<K>,
	K: keychain::Keychain,
{
	let prev = chain.head_header().unwrap();
	let fee_amt = txs.iter().map(|tx| tx.fee()).sum();
	let fees = BlockFees {
		fees: fee_amt,
		key_id: None,
		height: prev.height + 1,
	};
	let coinbase_tx = wallet::libwallet::internal::updater::receive_coinbase(wallet, &fees);
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
	let mut output = wallet.get(&fees.key_id.unwrap()).unwrap();
	output.block = block_id;
	output.merkle_proof = Some(MerkleProofWrapper(m_proof.unwrap()));
	let mut batch = wallet.batch().unwrap();
	batch.save(output).unwrap();
	batch.commit().unwrap();
}

/// adds many block rewards to a wallet, no transactions
pub fn award_blocks_to_wallet<T, K>(chain: &Chain, wallet: &mut T, num_rewards: usize)
where
	T: WalletBackend<K>,
	K: keychain::Keychain,
{
	for _ in 0..num_rewards {
		award_block_to_wallet(chain, vec![], wallet);
	}
}

/// Create a new wallet in a particular directory
pub fn create_wallet(dir: &str) -> FileWallet<ExtKeychain> {
	let mut wallet_config = WalletConfig::default();
	wallet_config.data_file_dir = String::from(dir);
	wallet::WalletSeed::init_file(&wallet_config).expect("Failed to create wallet seed file.");
	let mut wallet = FileWallet::new(wallet_config.clone(), "")
		.unwrap_or_else(|e| panic!("Error creating wallet: {:?} Config: {:?}", e, wallet_config));
	wallet.open_with_credentials().unwrap_or_else(|e| {
		panic!(
			"Error initializing wallet: {:?} Config: {:?}",
			e, wallet_config
		)
	});
	wallet
}
