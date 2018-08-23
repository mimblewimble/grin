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

//! tests for wallet restore
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate rand;
#[macro_use]
extern crate slog;
extern crate chrono;
extern crate serde;
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
use wallet::libtx::slate::Slate;
use wallet::libwallet;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

fn restore_wallet(
	base_dir: &str,
	wallet_dir: &str,
	backend_type: common::BackendType,
) -> Result<(), libwallet::Error> {
	let source_seed = format!("{}/{}/wallet.seed", base_dir, wallet_dir);
	let dest_dir = format!("{}/{}_restore", base_dir, wallet_dir);
	fs::create_dir_all(dest_dir.clone())?;
	let dest_seed = format!("{}/wallet.seed", dest_dir);
	fs::copy(source_seed, dest_seed)?;

	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(base_dir);
	let client = LocalWalletClient::new(wallet_dir, wallet_proxy.tx.clone());

	let wallet = common::create_wallet(&dest_dir, client.clone(), backend_type.clone());

	wallet_proxy.add_wallet(wallet_dir, client.get_send_instance(), wallet.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!(LOGGER, "Wallet Proxy error: {}", e);
		}
	});

	// perform the restore and update wallet info
	wallet::controller::owner_single_use(wallet.clone(), |api| {
		let _ = api.restore()?;
		let _ = api.retrieve_summary_info(true)?;
		Ok(())
	})?;

	Ok(())
}

fn compare_wallet_restore(
	base_dir: &str,
	wallet_dir: &str,
	backend_type: common::BackendType,
) -> Result<(), libwallet::Error> {
	let restore_name = format!("{}_restore", wallet_dir);
	let source_dir = format!("{}/{}", base_dir, wallet_dir);
	let dest_dir = format!("{}/{}", base_dir, restore_name);

	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(base_dir);

	let client = LocalWalletClient::new(wallet_dir, wallet_proxy.tx.clone());
	let wallet_source = common::create_wallet(&source_dir, client.clone(), backend_type.clone());
	wallet_proxy.add_wallet(
		&wallet_dir,
		client.get_send_instance(),
		wallet_source.clone(),
	);

	let client = LocalWalletClient::new(&restore_name, wallet_proxy.tx.clone());
	let wallet_dest = common::create_wallet(&dest_dir, client.clone(), backend_type.clone());
	wallet_proxy.add_wallet(
		&restore_name,
		client.get_send_instance(),
		wallet_dest.clone(),
	);

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!(LOGGER, "Wallet Proxy error: {}", e);
		}
	});

	let mut src_info: Option<libwallet::types::WalletInfo> = None;
	let mut dest_info: Option<libwallet::types::WalletInfo> = None;

	let mut src_txs: Option<Vec<libwallet::types::TxLogEntry>> = None;
	let mut dest_txs: Option<Vec<libwallet::types::TxLogEntry>> = None;

	// Overall wallet info should be the same
	wallet::controller::owner_single_use(wallet_source.clone(), |api| {
		src_info = Some(api.retrieve_summary_info(true)?.1);
		src_txs = Some(api.retrieve_txs(true, None)?.1);
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet_dest.clone(), |api| {
		dest_info = Some(api.retrieve_summary_info(true)?.1);
		dest_txs = Some(api.retrieve_txs(true, None)?.1);
		Ok(())
	})?;

	// Info should all be the same
	assert_eq!(src_info, dest_info);

	// Net differences in TX logs should be the same
	let src_sum: i64 = src_txs
		.unwrap()
		.iter()
		.map(|t| t.amount_credited as i64 - t.amount_debited as i64)
		.sum();

	let dest_sum: i64 = dest_txs
		.unwrap()
		.iter()
		.map(|t| t.amount_credited as i64 - t.amount_debited as i64)
		.sum();

	assert_eq!(src_sum, dest_sum);

	Ok(())
}

/// Build up 2 wallets, perform a few transactions on them
/// Then attempt to restore them in separate directories and check contents are the same
fn setup_restore(
	test_dir: &str,
	backend_type: common::BackendType,
) -> Result<(), libwallet::Error> {
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

	// Another wallet
	let client = LocalWalletClient::new("wallet3", wallet_proxy.tx.clone());
	let wallet3 = common::create_wallet(
		&format!("{}/wallet3", test_dir),
		client.clone(),
		backend_type.clone(),
	);
	wallet_proxy.add_wallet("wallet3", client.get_send_instance(), wallet3.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!(LOGGER, "Wallet Proxy error: {}", e);
		}
	});

	// mine a few blocks
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 10);

	// assert wallet contents
	// and a single use api for a send command
	let amount = 60_000_000_000;
	let mut slate = Slate::blank(1);
	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		slate = sender_api.issue_send_tx(
			amount,    // amount
			2,         // minimum confirmations
			"wallet2", // dest
			500,       // max outputs
			1,         // num change outputs
			true,      // select all outputs
		)?;
		sender_api.post_tx(&slate, false)?;
		Ok(())
	})?;

	// mine a few more blocks
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 3);

	// Send some to wallet 3
	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		slate = sender_api.issue_send_tx(
			amount * 2, // amount
			2,          // minimum confirmations
			"wallet3",  // dest
			500,        // max outputs
			1,          // num change outputs
			true,       // select all outputs
		)?;
		sender_api.post_tx(&slate, false)?;
		Ok(())
	})?;

	// mine a few more blocks
	let _ = common::award_blocks_to_wallet(&chain, wallet3.clone(), 10);

	// Wallet3 to wallet 2
	wallet::controller::owner_single_use(wallet3.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		slate = sender_api.issue_send_tx(
			amount * 3, // amount
			2,          // minimum confirmations
			"wallet2",  // dest
			500,        // max outputs
			1,          // num change outputs
			true,       // select all outputs
		)?;
		sender_api.post_tx(&slate, false)?;
		Ok(())
	})?;

	// mine a few more blocks
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 5);

	// update everyone
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let _ = api.retrieve_summary_info(true)?;
		Ok(())
	})?;
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let _ = api.retrieve_summary_info(true)?;
		Ok(())
	})?;
	wallet::controller::owner_single_use(wallet3.clone(), |api| {
		let _ = api.retrieve_summary_info(true)?;
		Ok(())
	})?;

	Ok(())
}

fn perform_restore(
	test_dir: &str,
	backend_type: common::BackendType,
) -> Result<(), libwallet::Error> {
	restore_wallet(test_dir, "wallet1", backend_type.clone())?;
	compare_wallet_restore(test_dir, "wallet1", backend_type.clone())?;
	restore_wallet(test_dir, "wallet2", backend_type.clone())?;
	compare_wallet_restore(test_dir, "wallet2", backend_type.clone())?;
	restore_wallet(test_dir, "wallet3", backend_type.clone())?;
	compare_wallet_restore(test_dir, "wallet3", backend_type)?;
	Ok(())
}

#[test]
fn db_wallet_restore() {
	let test_dir = "test_output/wallet_restore_db";
	if let Err(e) = setup_restore(test_dir, common::BackendType::LMDBBackend) {
		println!("Set up restore: Libwallet Error: {}", e);
	}
	if let Err(e) = perform_restore(test_dir, common::BackendType::LMDBBackend) {
		println!("Perform restore: Libwallet Error: {}", e);
	}
	// let logging finish
	thread::sleep(Duration::from_millis(200));
}
