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

//! tests for transactions building within core::libtx
#[macro_use]
extern crate log;
use self::core::global;
use self::core::global::ChainTypes;
use self::keychain::ExtKeychain;
use self::libwallet::slate::Slate;
use self::wallet::libwallet;
use self::wallet::libwallet::types::OutputStatus;
use self::wallet::test_framework::{self, LocalWalletClient, WalletProxy};
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

/// Exercises the Transaction API fully with a test NodeClient operating
/// directly on a chain instance
/// Callable with any type of wallet
fn basic_transaction_api(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	// Create a new wallet test client, and set its queues to communicate with the
	// proxy
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 =
		test_framework::create_wallet(&format!("{}/wallet1", test_dir), client1.clone(), None);
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	// define recipient wallet, add to proxy
	let wallet2 =
		test_framework::create_wallet(&format!("{}/wallet2", test_dir), client2.clone(), None);
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
									   // mine a few blocks
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 10);

	// Check wallet 1 contents are as expected
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		debug!(
			"Wallet 1 Info Pre-Transaction, after {} blocks: {:?}",
			wallet1_info.last_confirmed_height, wallet1_info
		);
		assert!(wallet1_refreshed);
		assert_eq!(
			wallet1_info.amount_currently_spendable,
			(wallet1_info.last_confirmed_height - cm) * reward
		);
		assert_eq!(wallet1_info.amount_immature, cm * reward);
		Ok(())
	})?;

	// assert wallet contents
	// and a single use api for a send command
	let amount = 60_000_000_000;
	let mut slate = Slate::blank(1);
	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		let (slate_i, lock_fn) = sender_api.initiate_tx(
			None, amount, // amount
			2,      // minimum confirmations
			1,      // num change outputs
			true,   // select all outputs
			None,
		)?;
		slate = client1.send_tx_slate_direct("wallet2", &slate_i)?;
		sender_api.tx_lock_outputs(&slate, lock_fn)?;
		sender_api.finalize_tx(&mut slate)?;
		Ok(())
	})?;

	// Check transaction log for wallet 1
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
		assert!(refreshed);
		let fee = core::libtx::tx_fee(
			wallet1_info.last_confirmed_height as usize - cm as usize,
			2,
			1,
			None,
		);
		// we should have a transaction entry for this slate
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		let tx = tx.unwrap();
		assert!(!tx.confirmed);
		assert!(tx.confirmation_ts.is_none());
		assert_eq!(tx.amount_debited - tx.amount_credited, fee + amount);
		assert_eq!(Some(fee), tx.fee);
		Ok(())
	})?;

	// Check transaction log for wallet 2
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
		assert!(refreshed);
		// we should have a transaction entry for this slate
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		let tx = tx.unwrap();
		assert!(!tx.confirmed);
		assert!(tx.confirmation_ts.is_none());
		assert_eq!(amount, tx.amount_credited);
		assert_eq!(0, tx.amount_debited);
		assert_eq!(None, tx.fee);
		Ok(())
	})?;

	// post transaction
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.post_tx(&slate.tx, false)?;
		Ok(())
	})?;

	// Check wallet 1 contents are as expected
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		debug!(
			"Wallet 1 Info Post Transaction, after {} blocks: {:?}",
			wallet1_info.last_confirmed_height, wallet1_info
		);
		let fee = core::libtx::tx_fee(
			wallet1_info.last_confirmed_height as usize - 1 - cm as usize,
			2,
			1,
			None,
		);
		assert!(wallet1_refreshed);
		// wallet 1 received fees, so amount should be the same
		assert_eq!(
			wallet1_info.total,
			amount * wallet1_info.last_confirmed_height - amount
		);
		assert_eq!(
			wallet1_info.amount_currently_spendable,
			(wallet1_info.last_confirmed_height - cm) * reward - amount - fee
		);
		assert_eq!(wallet1_info.amount_immature, cm * reward + fee);

		// check tx log entry is confirmed
		let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
		assert!(refreshed);
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		let tx = tx.unwrap();
		assert!(tx.confirmed);
		assert!(tx.confirmation_ts.is_some());

		Ok(())
	})?;

	// mine a few more blocks
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 3);

	// refresh wallets and retrieve info/tests for each wallet after maturity
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (wallet1_refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		debug!("Wallet 1 Info: {:?}", wallet1_info);
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
	})?;

	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.amount_currently_spendable, amount);

		// check tx log entry is confirmed
		let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
		assert!(refreshed);
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		let tx = tx.unwrap();
		assert!(tx.confirmed);
		assert!(tx.confirmation_ts.is_some());
		Ok(())
	})?;

	// Estimate fee and locked amount for a transaction
	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		let (total, fee) = sender_api.estimate_initiate_tx(
			None,
			amount * 2, // amount
			2,          // minimum confirmations
			1,          // num change outputs
			true,       // select all outputs
		)?;
		assert_eq!(total, 600_000_000_000);
		assert_eq!(fee, 4_000_000);

		let (total, fee) = sender_api.estimate_initiate_tx(
			None,
			amount * 2, // amount
			2,          // minimum confirmations
			1,          // num change outputs
			false,      // select the smallest amount of outputs
		)?;
		assert_eq!(total, 180_000_000_000);
		assert_eq!(fee, 6_000_000);

		Ok(())
	})?;

	// Send another transaction, but don't post to chain immediately and use
	// the stored transaction instead
	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		let (slate_i, lock_fn) = sender_api.initiate_tx(
			None,
			amount * 2, // amount
			2,          // minimum confirmations
			1,          // num change outputs
			true,       // select all outputs
			None,
		)?;
		slate = client1.send_tx_slate_direct("wallet2", &slate_i)?;
		sender_api.tx_lock_outputs(&slate, lock_fn)?;
		sender_api.finalize_tx(&mut slate)?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		let (refreshed, _wallet1_info) = sender_api.retrieve_summary_info(true, 1)?;
		assert!(refreshed);
		let (_, txs) = sender_api.retrieve_txs(true, None, None)?;
		// find the transaction
		let tx = txs
			.iter()
			.find(|t| t.tx_slate_id == Some(slate.id))
			.unwrap();
		let stored_tx = sender_api.get_stored_tx(&tx)?;
		sender_api.post_tx(&stored_tx.unwrap(), false)?;
		let (_, wallet1_info) = sender_api.retrieve_summary_info(true, 1)?;
		// should be mined now
		assert_eq!(
			wallet1_info.total,
			amount * wallet1_info.last_confirmed_height - amount * 3
		);
		Ok(())
	})?;

	// mine a few more blocks
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 3);

	// check wallet2 has stored transaction
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (wallet2_refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(wallet2_refreshed);
		assert_eq!(wallet2_info.amount_currently_spendable, amount * 3);

		// check tx log entry is confirmed
		let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
		assert!(refreshed);
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		let tx = tx.unwrap();
		assert!(tx.confirmed);
		assert!(tx.confirmation_ts.is_some());
		Ok(())
	})?;

	// Initiate transaction with weight more that
	// core::global::max_block_weight() should fail
	let invalid_amount_of_outputs =
		core::global::max_block_weight() / core::consensus::BLOCK_OUTPUT_WEIGHT + 1;
	let res = wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "posting"
		sender_api.initiate_tx(
			None,
			amount,                    // amount
			2,                         // minimum confirmations
			invalid_amount_of_outputs, // num change outputs
			true,                      // select all outputs
			None,
		)?;
		Ok(())
	});
	assert!(res.is_err());

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

/// Test rolling back transactions and outputs when a transaction is never
/// posted to a chain
fn tx_rollback(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	// Create a new wallet test client, and set its queues to communicate with the
	// proxy
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 =
		test_framework::create_wallet(&format!("{}/wallet1", test_dir), client1.clone(), None);
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	// define recipient wallet, add to proxy
	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	let wallet2 =
		test_framework::create_wallet(&format!("{}/wallet2", test_dir), client2.clone(), None);
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
									   // mine a few blocks
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 5);

	let amount = 30_000_000_000;
	let mut slate = Slate::blank(1);
	wallet::controller::owner_single_use(wallet1.clone(), |sender_api| {
		// note this will increment the block count as part of the transaction "Posting"
		let (slate_i, lock_fn) = sender_api.initiate_tx(
			None, amount, // amount
			2,      // minimum confirmations
			1,      // num change outputs
			true,   // select all outputs
			None,
		)?;
		slate = client1.send_tx_slate_direct("wallet2", &slate_i)?;
		sender_api.tx_lock_outputs(&slate, lock_fn)?;
		sender_api.finalize_tx(&mut slate)?;
		Ok(())
	})?;

	// Check transaction log for wallet 1
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		println!(
			"last confirmed height: {}",
			wallet1_info.last_confirmed_height
		);
		assert!(refreshed);
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		// we should have a transaction entry for this slate
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		let mut locked_count = 0;
		let mut unconfirmed_count = 0;
		// get the tx entry, check outputs are as expected
		let (_, outputs) = api.retrieve_outputs(true, false, Some(tx.unwrap().id))?;
		for (o, _) in outputs.clone() {
			if o.status == OutputStatus::Locked {
				locked_count = locked_count + 1;
			}
			if o.status == OutputStatus::Unconfirmed {
				unconfirmed_count = unconfirmed_count + 1;
			}
		}
		assert_eq!(outputs.len(), 3);
		assert_eq!(locked_count, 2);
		assert_eq!(unconfirmed_count, 1);

		Ok(())
	})?;

	// Check transaction log for wallet 2
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
		assert!(refreshed);
		let mut unconfirmed_count = 0;
		let tx = txs.iter().find(|t| t.tx_slate_id == Some(slate.id));
		assert!(tx.is_some());
		// get the tx entry, check outputs are as expected
		let (_, outputs) = api.retrieve_outputs(true, false, Some(tx.unwrap().id))?;
		for (o, _) in outputs.clone() {
			if o.status == OutputStatus::Unconfirmed {
				unconfirmed_count = unconfirmed_count + 1;
			}
		}
		assert_eq!(outputs.len(), 1);
		assert_eq!(unconfirmed_count, 1);
		let (refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(refreshed);
		assert_eq!(wallet2_info.amount_currently_spendable, 0,);
		assert_eq!(wallet2_info.total, amount);
		Ok(())
	})?;

	// wallet 1 is bold and doesn't ever post the transaction mine a few more blocks
	let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 5);

	// Wallet 1 decides to roll back instead
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		// can't roll back coinbase
		let res = api.cancel_tx(Some(1), None);
		assert!(res.is_err());
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		let tx = txs
			.iter()
			.find(|t| t.tx_slate_id == Some(slate.id))
			.unwrap();
		api.cancel_tx(Some(tx.id), None)?;
		let (refreshed, wallet1_info) = api.retrieve_summary_info(true, 1)?;
		assert!(refreshed);
		println!(
			"last confirmed height: {}",
			wallet1_info.last_confirmed_height
		);
		// check all eligible inputs should be now be spendable
		println!("cm: {}", cm);
		assert_eq!(
			wallet1_info.amount_currently_spendable,
			(wallet1_info.last_confirmed_height - cm) * reward
		);
		// can't roll back again
		let res = api.cancel_tx(Some(tx.id), None);
		assert!(res.is_err());

		Ok(())
	})?;

	// Wallet 2 rolls back
	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let (_, txs) = api.retrieve_txs(true, None, None)?;
		let tx = txs
			.iter()
			.find(|t| t.tx_slate_id == Some(slate.id))
			.unwrap();
		api.cancel_tx(Some(tx.id), None)?;
		let (refreshed, wallet2_info) = api.retrieve_summary_info(true, 1)?;
		assert!(refreshed);
		// check all eligible inputs should be now be spendable
		assert_eq!(wallet2_info.amount_currently_spendable, 0,);
		assert_eq!(wallet2_info.total, 0,);
		// can't roll back again
		let res = api.cancel_tx(Some(tx.id), None);
		assert!(res.is_err());

		Ok(())
	})?;

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

#[test]
fn db_wallet_basic_transaction_api() {
	let test_dir = "test_output/basic_transaction_api";
	if let Err(e) = basic_transaction_api(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}

#[test]
fn db_wallet_tx_rollback() {
	let test_dir = "test_output/tx_rollback";
	if let Err(e) = tx_rollback(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}
