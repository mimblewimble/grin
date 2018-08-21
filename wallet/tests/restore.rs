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
use wallet::libwallet::types::OutputStatus;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

/// Build up 2 wallets, perform a few transactions on them
/// Then attempt to restore them in separate directories and check contents are the same
fn restore(test_dir: &str, backend_type: common::BackendType) -> Result<(), libwallet::Error> {
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
	wallet_proxy.add_wallet("wallet3", client.get_send_instance(), wallet2.clone());

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

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

#[test]
fn db_wallet_restore() {
	let test_dir = "test_output/wallet_restore_db";
	if let Err(e) = restore(test_dir, common::BackendType::LMDBBackend) {
		println!("Libwallet Error: {}", e);
	}
}
