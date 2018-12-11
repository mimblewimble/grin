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

//! Test a wallet repost command
#[macro_use]
extern crate log;

use self::core::global;
use self::core::global::ChainTypes;
use self::core::libtx::slate::Slate;
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

/// self send impl
fn file_repost_test_impl(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 = test_framework::create_wallet(&format!("{}/wallet1", test_dir), client1.clone());
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
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

	// add some accounts
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.create_account_path("mining")?;
		api.create_account_path("listener")?;
		Ok(())
	})?;

	// add some accounts
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		api.create_account_path("account1")?;
		api.create_account_path("account2")?;
		Ok(())
	})?;

	// Get some mining done
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("mining")?;
	}
	let mut bh = 10u64;
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), bh as usize);

	let send_file = format!("{}/part_tx_1.tx", test_dir);
	let receive_file = format!("{}/part_tx_2.tx", test_dir);

	// Should have 5 in account1 (5 spendable), 5 in account (2 spendable)
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, bh);
		assert_eq!(wallet1_info.total, bh * reward);
		// send to send
		let (mut slate, lock_fn) = api.initiate_tx(
			Some("mining"),
			reward * 2, // amount
			2,          // minimum confirmations
			500,        // max outputs
			1,          // num change outputs
			true,       // select all outputs
			None,
		)?;
		// output tx file
		let file_adapter = FileWalletCommAdapter::new();
		file_adapter.send_tx_async(&send_file, &mut slate)?;
		api.tx_lock_outputs(&slate, lock_fn)?;
		Ok(())
	})?;

	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
	bh += 3;

	// wallet 1 receives file to different account, completes
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("listener")?;
	}

	wallet::controller::foreign_single_use(wallet1.clone(), |api| {
		let adapter = FileWalletCommAdapter::new();
		let mut slate = adapter.receive_tx_async(&send_file)?;
		api.receive_tx(&mut slate, None, None)?;
		adapter.send_tx_async(&receive_file, &mut slate)?;
		Ok(())
	})?;

	// wallet 1 receives file to different account, completes
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("mining")?;
	}

	let mut slate = Slate::blank(2);
	// wallet 1 finalize
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let adapter = FileWalletCommAdapter::new();
		slate = adapter.receive_tx_async(&receive_file)?;
		api.finalize_tx(&mut slate)?;
		Ok(())
	})?;

	// Now repost from cached
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, txs) = api.retrieve_txs(true, None, Some(slate.id))?;
		api.post_tx(&txs[0].get_stored_tx().unwrap(), false)?;
		bh += 1;
		Ok(())
	})?;

	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
	bh += 3;

	// update/test contents of both accounts
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, bh);
		assert_eq!(wallet1_info.total, bh * reward - reward * 2);
		Ok(())
	})?;

	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("listener")?;
	}

	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.last_confirmed_height, bh);
		assert_eq!(wallet2_info.total, 2 * reward);
		Ok(())
	})?;

	// as above, but syncronously
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("mining")?;
	}
	{
		let mut w = wallet2.lock();
		w.set_parent_key_id_by_name("account1")?;
	}

	let mut slate = Slate::blank(2);
	let amount = 60_000_000_000;

	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		let (slate_i, lock_fn) = sender_api.initiate_tx(
			None,
			amount * 2, // amount
			2,          // minimum confirmations
			500,        // max outputs
			1,          // num change outputs
			true,       // select all outputs
			None,
		)?;
		slate = client1.send_tx_slate_direct("wallet2", &slate_i)?;
		sender_api.tx_lock_outputs(&slate, lock_fn)?;
		sender_api.finalize_tx(&mut slate)?;
		Ok(())
	})?;

	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
	bh += 3;

	// Now repost from cached
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, txs) = api.retrieve_txs(true, None, Some(slate.id))?;
		api.post_tx(&txs[0].get_stored_tx().unwrap(), false)?;
		bh += 1;
		Ok(())
	})?;

	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
	bh += 3;
	//
	// update/test contents of both accounts
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet1_refreshed);
		assert_eq!(wallet1_info.last_confirmed_height, bh);
		assert_eq!(wallet1_info.total, bh * reward - reward * 4);
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.last_confirmed_height, bh);
		assert_eq!(wallet2_info.total, 2 * amount);
		Ok(())
	})?;

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

#[test]
fn wallet_file_repost() {
	let test_dir = "test_output/file_repost";
	if let Err(e) = file_repost_test_impl(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}
