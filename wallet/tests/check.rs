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

//! tests differing accounts in the same wallet
#[macro_use]
extern crate log;

use self::core::global;
use self::core::global::ChainTypes;
use self::keychain::ExtKeychain;
use self::wallet::test_framework::{self, LocalWalletClient, WalletProxy};
use self::wallet::{libwallet, FileWalletCommAdapter};
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use grin_wallet as wallet;
use std::fs;
use std::thread;
use std::time::Duration;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

/// Various tests on accounts within the same wallet
fn check_repair_impl(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	// Create a new wallet test client, and set its queues to communicate with the
	// proxy
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 = test_framework::create_wallet(&format!("{}/wallet1", test_dir), client1.clone());
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	// define recipient wallet, add to proxy
	let wallet2 = test_framework::create_wallet(&format!("{}/wallet2", test_dir), client2.clone());
	wallet_proxy.add_wallet("wallet2", client2.get_send_instance(), wallet2.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!("Wallet Proxy error: {}", e);
		}
	});

	// few values to keep things shorter
	let reward = core::consensus::REWARD;
	let cm = global::coinbase_maturity(); // assume all testing precedes soft fork height

	// add some accounts
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.create_account_path("account_1")?;
		api.create_account_path("account_2")?;
		api.create_account_path("account_3")?;
		api.set_active_account("account_1")?;
		Ok(())
	})?;

	// add account to wallet 2
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		api.create_account_path("account_1")?;
		api.set_active_account("account_1")?;
		Ok(())
	})?;

	// Do some mining
	let bh = 20u64;
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), bh as usize);

	// Sanity check contents
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, bh);
		assert_eq!(wallet1_info.total, bh * reward);
		assert_eq!(wallet1_info.amount_currently_spendable, (bh - cm) * reward);
		// check tx log as well
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		let (c, _) = libwallet::types::TxLogEntry::sum_confirmed(&txs);
		assert_eq!(wallet1_info.total, c);
		assert_eq!(txs.len(), bh as usize);
		Ok(())
	})?;

	// Accidentally delete some outputs
	let mut w1_outputs_commits = vec![];
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		w1_outputs_commits = api.retrieve_outputs(false, true, None)?.1;
		Ok(())
	})?;
	let w1_outputs: Vec<libwallet::types::OutputData> =
		w1_outputs_commits.into_iter().map(|o| o.0).collect();
	{
		let mut w = wallet1.lock();
		w.open_with_credentials()?;
		{
			let mut batch = w.batch()?;
			batch.delete(&w1_outputs[4].key_id)?;
			batch.delete(&w1_outputs[10].key_id)?;
			let mut accidental_spent = w1_outputs[13].clone();
			accidental_spent.status = libwallet::types::OutputStatus::Spent;
			batch.save(accidental_spent)?;
			batch.commit()?;
		}
		w.close()?;
	}

	// check we have a problem now
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		let (c, _) = libwallet::types::TxLogEntry::sum_confirmed(&txs);
		assert!(wallet1_info.total != c);
		Ok(())
	})?;

	// this should restore our missing outputs
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.check_repair()?;
		Ok(())
	})?;

	// check our outputs match again
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.total, bh * reward);
		Ok(())
	})?;

	// perform a transaction, but don't let it finish
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		// send to send
		let (mut slate, lock_fn) = api.initiate_tx(
			None,
			reward * 2, // amount
			cm,         // minimum confirmations
			500,        // max outputs
			1,          // num change outputs
			true,       // select all outputs
			None,       // optional message
		)?;
		// output tx file
		let file_adapter = FileWalletCommAdapter::new();
		let send_file = format!("{}/part_tx_1.tx", test_dir);
		file_adapter.send_tx_async(&send_file, &mut slate)?;
		api.tx_lock_outputs(&slate, lock_fn)?;
		Ok(())
	})?;

	// check we're all locked
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_info.amount_currently_spendable == 0);
		Ok(())
	})?;

	// unlock/restore
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.check_repair()?;
		Ok(())
	})?;

	// check spendable amount again
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert_eq!(wallet1_info.amount_currently_spendable, (bh - cm) * reward);
		Ok(())
	})?;

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

#[test]
fn check_repair() {
	let test_dir = "test_output/check_repair";
	if let Err(e) = check_repair_impl(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}
