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

extern crate chrono;
extern crate failure;
extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_wallet as wallet;
extern crate serde_json;

use chrono::Duration;
use std::sync::{Arc, Mutex};

use chain::Chain;
use core::core::{OutputFeatures, OutputIdentifier, Transaction};
use core::{consensus, global, pow, ser};
use wallet::libwallet;
use wallet::libwallet::types::{BlockFees, CbData, WalletClient, WalletInst};
use wallet::lmdb_wallet::LMDBBackend;
use wallet::WalletConfig;

use util;
use util::secp::pedersen;

pub mod testclient;

/// types of backends tests should iterate through
#[derive(Clone)]
pub enum BackendType {
	/// File
	FileBackend,
	/// LMDB
	LMDBBackend,
}

/// Get an output from the chain locally and present it back as an API output
fn get_output_local(chain: &chain::Chain, commit: &pedersen::Commitment) -> Option<api::Output> {
	let outputs = [
		OutputIdentifier::new(OutputFeatures::DEFAULT_OUTPUT, commit),
		OutputIdentifier::new(OutputFeatures::COINBASE_OUTPUT, commit),
	];

	for x in outputs.iter() {
		if let Ok(_) = chain.is_unspent(&x) {
			let block_height = chain.get_header_for_output(&x).unwrap().height;
			return Some(api::Output::new(&commit, block_height));
		}
	}
	None
}

/// get output listing traversing pmmr from local
fn get_outputs_by_pmmr_index_local(
	chain: Arc<chain::Chain>,
	start_index: u64,
	max: u64,
) -> api::OutputListing {
	let outputs = chain
		.unspent_outputs_by_insertion_index(start_index, max)
		.unwrap();
	api::OutputListing {
		last_retrieved_index: outputs.0,
		highest_index: outputs.1,
		outputs: outputs
			.2
			.iter()
			.map(|x| api::OutputPrintable::from_output(x, chain.clone(), None, true))
			.collect(),
	}
}

/// Adds a block with a given reward to the chain and mines it
pub fn add_block_with_reward(chain: &Chain, txs: Vec<&Transaction>, reward: CbData) {
	let prev = chain.head_header().unwrap();
	let next_header_info = consensus::next_difficulty(1, chain.difficulty_iter());
	let out_bin = util::from_hex(reward.output).unwrap();
	let kern_bin = util::from_hex(reward.kernel).unwrap();
	let output = ser::deserialize(&mut &out_bin[..]).unwrap();
	let kernel = ser::deserialize(&mut &kern_bin[..]).unwrap();
	let mut b = core::core::Block::new(
		&prev,
		txs.into_iter().cloned().collect(),
		next_header_info.clone().difficulty,
		(output, kernel),
	).unwrap();
	b.header.timestamp = prev.timestamp + Duration::seconds(60);
	b.header.pow.scaling_difficulty = next_header_info.secondary_scaling;
	chain.set_txhashset_roots(&mut b, false).unwrap();
	pow::pow_size(
		&mut b.header,
		next_header_info.difficulty,
		global::proofsize(),
		global::min_edge_bits(),
	).unwrap();
	chain.process_block(b, chain::Options::MINE).unwrap();
	chain.validate(false).unwrap();
}

/// adds a reward output to a wallet, includes that reward in a block, mines
/// the block and adds it to the chain, with option transactions included.
/// Helpful for building up precise wallet balances for testing.
pub fn award_block_to_wallet<C, K>(
	chain: &Chain,
	txs: Vec<&Transaction>,
	wallet: Arc<Mutex<Box<WalletInst<C, K>>>>,
) -> Result<(), libwallet::Error>
where
	C: WalletClient,
	K: keychain::Keychain,
{
	// build block fees
	let prev = chain.head_header().unwrap();
	let fee_amt = txs.iter().map(|tx| tx.fee()).sum();
	let block_fees = BlockFees {
		fees: fee_amt,
		key_id: None,
		height: prev.height + 1,
	};
	// build coinbase (via api) and add block
	libwallet::controller::foreign_single_use(wallet.clone(), |api| {
		let coinbase_tx = api.build_coinbase(&block_fees)?;
		add_block_with_reward(chain, txs, coinbase_tx.clone());
		Ok(())
	})?;
	Ok(())
}

/// Award a blocks to a wallet directly
pub fn award_blocks_to_wallet<C, K>(
	chain: &Chain,
	wallet: Arc<Mutex<Box<WalletInst<C, K>>>>,
	number: usize,
) -> Result<(), libwallet::Error>
where
	C: WalletClient,
	K: keychain::Keychain,
{
	for _ in 0..number {
		award_block_to_wallet(chain, vec![], wallet.clone())?;
	}
	Ok(())
}

/// dispatch a db wallet
pub fn create_wallet<C, K>(dir: &str, client: C) -> Arc<Mutex<Box<WalletInst<C, K>>>>
where
	C: WalletClient + 'static,
	K: keychain::Keychain + 'static,
{
	let mut wallet_config = WalletConfig::default();
	wallet_config.data_file_dir = String::from(dir);
	let _ = wallet::WalletSeed::init_file(&wallet_config);
	let mut wallet: Box<WalletInst<C, K>> = {
		let mut wallet: LMDBBackend<C, K> = LMDBBackend::new(wallet_config.clone(), "", client)
			.unwrap_or_else(|e| {
				panic!("Error creating wallet: {:?} Config: {:?}", e, wallet_config)
			});
		Box::new(wallet)
	};
	wallet.open_with_credentials().unwrap_or_else(|e| {
		panic!(
			"Error initializing wallet: {:?} Config: {:?}",
			e, wallet_config
		)
	});
	Arc::new(Mutex::new(wallet))
}
