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

extern crate failure;
extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_wallet as wallet;
extern crate serde_json;
extern crate time;

use std::sync::{Arc, Mutex};

use chain::Chain;
use core::core::{OutputFeatures, OutputIdentifier, Transaction};
use core::{consensus, global, pow, ser};
use wallet::file_wallet::FileWallet;
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
			return Some(api::Output::new(&commit));
		}
	}
	None
}

/// Adds a block with a given reward to the chain and mines it
pub fn add_block_with_reward(chain: &Chain, txs: Vec<&Transaction>, reward: CbData) {
	let prev = chain.head_header().unwrap();
	let difficulty = consensus::next_difficulty(chain.difficulty_iter()).unwrap();
	let out_bin = util::from_hex(reward.output).unwrap();
	let kern_bin = util::from_hex(reward.kernel).unwrap();
	let output = ser::deserialize(&mut &out_bin[..]).unwrap();
	let kernel = ser::deserialize(&mut &kern_bin[..]).unwrap();
	let mut b = core::core::Block::new(
		&prev,
		txs.into_iter().cloned().collect(),
		difficulty.clone(),
		(output, kernel),
	).unwrap();
	b.header.timestamp = prev.timestamp + time::Duration::seconds(60);
	chain.set_txhashset_roots(&mut b, false).unwrap();
	pow::pow_size(
		&mut b.header,
		difficulty,
		global::proofsize(),
		global::min_sizeshift(),
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

/// dispatch a wallet (extend later to optionally dispatch a db wallet)
pub fn create_wallet<C, K>(
	dir: &str,
	client: C,
	backend_type: BackendType,
) -> Arc<Mutex<Box<WalletInst<C, K>>>>
where
	C: WalletClient + 'static,
	K: keychain::Keychain + 'static,
{
	let mut wallet_config = WalletConfig::default();
	wallet_config.data_file_dir = String::from(dir);
	let _ = wallet::WalletSeed::init_file(&wallet_config);
	let mut wallet: Box<WalletInst<C, K>> = match backend_type {
		BackendType::FileBackend => {
			let mut wallet: FileWallet<C, K> = FileWallet::new(wallet_config.clone(), "", client)
				.unwrap_or_else(|e| {
					panic!("Error creating wallet: {:?} Config: {:?}", e, wallet_config)
				});
			Box::new(wallet)
		}
		BackendType::LMDBBackend => {
			let mut wallet: LMDBBackend<C, K> = LMDBBackend::new(wallet_config.clone(), "", client)
				.unwrap_or_else(|e| {
					panic!("Error creating wallet: {:?} Config: {:?}", e, wallet_config)
				});
			Box::new(wallet)
		}
	};
	wallet.open_with_credentials().unwrap_or_else(|e| {
		panic!(
			"Error initializing wallet: {:?} Config: {:?}",
			e, wallet_config
		)
	});
	Arc::new(Mutex::new(wallet))
}
