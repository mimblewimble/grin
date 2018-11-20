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
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate rand;
#[macro_use]
extern crate log;
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
use keychain::{ExtKeychain, Keychain};
use wallet::libwallet;

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

/// Various tests on accounts within the same wallet
fn accounts_test_impl(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	// Create a new wallet test client, and set its queues to communicate with the
	// proxy
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 = common::create_wallet(&format!("{}/wallet1", test_dir), client1.clone());
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	// define recipient wallet, add to proxy
	let wallet2 = common::create_wallet(&format!("{}/wallet2", test_dir), client2.clone());
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

	// test default accounts exist
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let accounts = api.accounts()?;
		assert_eq!(accounts[0].label, "default");
		assert_eq!(accounts[0].path, ExtKeychain::derive_key_id(2, 0, 0, 0, 0));
		Ok(())
	})?;

	// add some accounts
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let new_path = api.new_account_path("account1").unwrap();
		assert_eq!(new_path, ExtKeychain::derive_key_id(2, 1, 0, 0, 0));
		let new_path = api.new_account_path("account2").unwrap();
		assert_eq!(new_path, ExtKeychain::derive_key_id(2, 2, 0, 0, 0));
		let new_path = api.new_account_path("account3").unwrap();
		assert_eq!(new_path, ExtKeychain::derive_key_id(2, 3, 0, 0, 0));
		// trying to add same label again should fail
		let res = api.new_account_path("account1");
		assert!(res.is_err());
		Ok(())
	})?;

	// add account to wallet 2
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let new_path = api.new_account_path("listener_account").unwrap();
		assert_eq!(new_path, ExtKeychain::derive_key_id(2, 1, 0, 0, 0));
		Ok(())
	})?;

	// Default wallet 2 to listen on that account
	{
		let mut w = wallet2.lock();
		w.set_parent_key_id_by_name("listener_account")?;
	}

	// Mine into two different accounts in the same wallet
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("account1")?;
		assert_eq!(w.parent_key_id(), ExtKeychain::derive_key_id(2, 1, 0, 0, 0));
	}
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 7);

	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("account2")?;
		assert_eq!(w.parent_key_id(), ExtKeychain::derive_key_id(2, 2, 0, 0, 0));
	}
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 5);

	// Should have 5 in account1 (5 spendable), 5 in account (2 spendable)
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, 12);
		assert_eq!(wallet1_info.total, 5 * reward);
		assert_eq!(wallet1_info.amount_currently_spendable, (5 - cm) * reward);
		// check tx log as well
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		assert_eq!(txs.len(), 5);
		Ok(())
	})?;
	// now check second account
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("account1")?;
	}
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		// check last confirmed height on this account is different from above (should be 0)
		let (_, wallet1_info) = api.retrieve_summary_info(false, 1)?;
		assert_eq!(wallet1_info.last_confirmed_height, 0);
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, 12);
		assert_eq!(wallet1_info.total, 7 * reward);
		assert_eq!(wallet1_info.amount_currently_spendable, 7 * reward);
		// check tx log as well
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		assert_eq!(txs.len(), 7);
		Ok(())
	})?;

	// should be nothing in default account
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("default")?;
	}
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, wallet1_info) = api.retrieve_summary_info(false, 1)?;
		assert_eq!(wallet1_info.last_confirmed_height, 0);
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, 12);
		assert_eq!(wallet1_info.total, 0,);
		assert_eq!(wallet1_info.amount_currently_spendable, 0,);
		// check tx log as well
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		assert_eq!(txs.len(), 0);
		Ok(())
	})?;

	// Send a tx to another wallet
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("account1")?;
	}

	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (mut slate, lock_fn) = api.initiate_tx(
			None, reward, // amount
			2,      // minimum confirmations
			500,    // max outputs
			1,      // num change outputs
			true,   // select all outputs
		)?;
		slate = client1.send_tx_slate_direct("wallet2", &slate)?;
		api.finalize_tx(&mut slate)?;
		api.tx_lock_outputs(&slate, lock_fn)?;
		api.post_tx(&slate, false)?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, 13);
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		assert_eq!(txs.len(), 9);
		Ok(())
	})?;

	// other account should be untouched
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("account2")?;
	}
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, wallet1_info) = api.retrieve_summary_info(false, 1)?;
		assert_eq!(wallet1_info.last_confirmed_height, 12);
		let (_, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert_eq!(wallet1_info.last_confirmed_height, 13);
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		println!("{:?}", txs);
		assert_eq!(txs.len(), 5);
		Ok(())
	})?;

	// wallet 2 should only have this tx on the listener account
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.last_confirmed_height, 13);
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		assert_eq!(txs.len(), 1);
		Ok(())
	})?;
	// Default account on wallet 2 should be untouched
	{
		let mut w = wallet2.lock();
		w.set_parent_key_id_by_name("default")?;
	}
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (_, wallet2_info) = api.retrieve_summary_info(false, 1)?;
		assert_eq!(wallet2_info.last_confirmed_height, 0);
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.last_confirmed_height, 13);
		assert_eq!(wallet2_info.total, 0,);
		assert_eq!(wallet2_info.amount_currently_spendable, 0,);
		// check tx log as well
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		assert_eq!(txs.len(), 0);
		Ok(())
	})?;

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

#[test]
fn accounts() {
	let test_dir = "test_output/accounts";
	if let Err(e) = accounts_test_impl(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}
