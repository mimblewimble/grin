// Copyright 2018 The Grin Developers
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

//! tests for transactions building within libtx
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate rand;
#[macro_use]
extern crate slog;
extern crate serde;
extern crate time;
extern crate uuid;

mod common;
use common::testclient::{LocalWalletClient, WalletProxy};

use std::fs;
use std::thread;
use std::time::Duration;

use core::global;
use core::global::ChainTypes;
use keychain::ExtKeychain;
use util::LOGGER;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

/// Exercises the Transaction API fully with a test WalletClient operating
/// directly on a chain instance
/// Callable with any type of wallet
fn basic_transaction_api(test_dir: &str, backend_type: common::BackendType) {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	// Create a new wallet test client, and set its queues to communicate with the
	// proxy
	let client = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 = common::create_wallet(
		&format!("{}/wallet1", test_dir),
		client.clone(),
		backend_type.clone(),
	);
	wallet_proxy.add_wallet("wallet1", client.get_send_instance(), wallet1.clone());

	// define recipient wallet, add to proxy
	let client = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	let wallet2 = common::create_wallet(
		&format!("{}/wallet2", test_dir),
		client.clone(),
		backend_type.clone(),
	);
	wallet_proxy.add_wallet("wallet2", client.get_send_instance(), wallet2.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!(LOGGER, "Wallet Proxy error: {}", e);
		}
	});

	// few values to keep things shorter
	let reward = core::consensus::REWARD;
	let cm = global::coinbase_maturity();
	// mine a few blocks
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 10);

	// Check wallet 1 contents are as expected
	let sender_res = wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true)?;
		debug!(
			LOGGER,
			"Wallet 1 Info Pre-Transaction, after {} blocks: {:?}",
			wallet1_info.last_confirmed_height,
			wallet1_info
		);
		assert!(wallet1_refreshed);
		assert_eq!(
			wallet1_info.amount_currently_spendable,
			(wallet1_info.last_confirmed_height - cm) * reward
		);
		assert_eq!(wallet1_info.amount_immature, cm * reward);
		Ok(())
	});
	if let Err(e) = sender_res {
		println!("Error starting sender API: {}", e);
	}

	// assert wallet contents
	// and a single use api for a send command
	let amount = 60_000_000_000;
	let sender_res = wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		let issue_tx_res = sender_api.issue_send_tx(
			amount,    // amount
			2,         // minimum confirmations
			"wallet2", // dest
			500,       // max outputs
			true,      // select all outputs
		);
		if issue_tx_res.is_err() {
			panic!("Error issuing send tx: {}", issue_tx_res.err().unwrap());
		}
		let post_res = sender_api.post_tx(&issue_tx_res.unwrap(), false);
		if post_res.is_err() {
			panic!("Error posting tx: {}", post_res.err().unwrap());
		}

		Ok(())
	});
	if let Err(e) = sender_res {
		panic!("Error starting sender API: {}", e);
	}

	// Check wallet 1 contents are as expected
	let sender_res = wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true)?;
		debug!(
			LOGGER,
			"Wallet 1 Info Post Transaction, after {} blocks: {:?}",
			wallet1_info.last_confirmed_height,
			wallet1_info
		);
		let fee = wallet::libtx::tx_fee(
			wallet1_info.last_confirmed_height as usize - 1 - cm as usize,
			2,
			None,
		);
		assert!(wallet1_refreshed);
		// wallet 1 recieved fees, so amount should be the same
		assert_eq!(
			wallet1_info.total,
			amount * wallet1_info.last_confirmed_height - amount
		);
		assert_eq!(
			wallet1_info.amount_currently_spendable,
			(wallet1_info.last_confirmed_height - cm) * reward - amount - fee
		);
		assert_eq!(wallet1_info.amount_immature, cm * reward + fee);
		Ok(())
	});
	if let Err(e) = sender_res {
		println!("Error starting sender API: {}", e);
	}

	// mine a few more blocks
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 3);

	// refresh wallets and retrieve info/tests for each wallet after maturity
	let sender_res = wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true)?;
		debug!(LOGGER, "Wallet 1 Info: {:?}", wallet1_info);
		assert!(wallet1_refreshed);
		assert_eq!(
			wallet1_info.total,
			amount * wallet1_info.last_confirmed_height - amount
		);
		assert_eq!(
			wallet1_info.amount_currently_spendable,
			(wallet1_info.last_confirmed_height - cm - 1) * reward
		);
		Ok(())
	});
	if let Err(e) = sender_res {
		println!("Error starting sender API: {}", e);
	}

	let sender_res = wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.amount_currently_spendable, amount);
		Ok(())
	});
	if let Err(e) = sender_res {
		println!("Error starting sender API: {}", e);
	}

	// let logging finish
	thread::sleep(Duration::from_millis(200));
}

#[test]
fn file_wallet_basic_transaction_api() {
	let test_dir = "test_output/basic_transaction_api_file";
	basic_transaction_api(test_dir, common::BackendType::FileBackend);
}

// not yet ready
#[ignore]
#[test]
fn db_wallet_basic_transaction_api() {
	let test_dir = "test_output/basic_transaction_api_db";
	basic_transaction_api(test_dir, common::BackendType::LMDBBackend);
}
