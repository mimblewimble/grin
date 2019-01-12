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

use self::core::consensus;
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

/// Various tests on checking functionality
fn check_repair_impl(test_dir: &str) -> Result<(), libwallet::Error> {
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
	let cm = global::coinbase_maturity() as u64; // assume all testing precedes soft fork height

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
			batch.delete(&w1_outputs[4].key_id, &None)?;
			batch.delete(&w1_outputs[10].key_id, &None)?;
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

fn two_wallets_one_seed_impl(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	let seed_phrase =
		"affair pistol cancel crush garment candy ancient flag work \
		 market crush dry stand focus mutual weapon offer ceiling rival turn team spring \
		 where swift";

	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();

	// Create a new wallet test client, and set its queues to communicate with the
	// proxy
	let m_client = LocalWalletClient::new("miner", wallet_proxy.tx.clone());
	let miner =
		test_framework::create_wallet(&format!("{}/miner", test_dir), m_client.clone(), None);
	wallet_proxy.add_wallet("miner", m_client.get_send_instance(), miner.clone());

	// non-mining recipient wallets
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let wallet1 = test_framework::create_wallet(
		&format!("{}/wallet1", test_dir),
		client1.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	let wallet2 = test_framework::create_wallet(
		&format!("{}/wallet2", test_dir),
		client2.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet2", client2.get_send_instance(), wallet2.clone());

	// we'll restore into here
	let client3 = LocalWalletClient::new("wallet3", wallet_proxy.tx.clone());
	let wallet3 = test_framework::create_wallet(
		&format!("{}/wallet3", test_dir),
		client3.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet3", client3.get_send_instance(), wallet3.clone());

	// also restore into here
	let client4 = LocalWalletClient::new("wallet4", wallet_proxy.tx.clone());
	let wallet4 = test_framework::create_wallet(
		&format!("{}/wallet4", test_dir),
		client4.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet4", client4.get_send_instance(), wallet4.clone());

	// Simulate a recover from seed without restore into here
	let client5 = LocalWalletClient::new("wallet5", wallet_proxy.tx.clone());
	let wallet5 = test_framework::create_wallet(
		&format!("{}/wallet5", test_dir),
		client5.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet5", client5.get_send_instance(), wallet5.clone());

	//simulate a recover from seed without restore into here
	let client6 = LocalWalletClient::new("wallet6", wallet_proxy.tx.clone());
	let wallet6 = test_framework::create_wallet(
		&format!("{}/wallet6", test_dir),
		client6.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet6", client6.get_send_instance(), wallet6.clone());

	let client7 = LocalWalletClient::new("wallet7", wallet_proxy.tx.clone());
	let wallet7 = test_framework::create_wallet(
		&format!("{}/wallet7", test_dir),
		client7.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet7", client7.get_send_instance(), wallet7.clone());

	let client8 = LocalWalletClient::new("wallet8", wallet_proxy.tx.clone());
	let wallet8 = test_framework::create_wallet(
		&format!("{}/wallet8", test_dir),
		client8.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet8", client8.get_send_instance(), wallet8.clone());

	let client9 = LocalWalletClient::new("wallet9", wallet_proxy.tx.clone());
	let wallet9 = test_framework::create_wallet(
		&format!("{}/wallet9", test_dir),
		client9.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet9", client9.get_send_instance(), wallet9.clone());

	let client10 = LocalWalletClient::new("wallet10", wallet_proxy.tx.clone());
	let wallet10 = test_framework::create_wallet(
		&format!("{}/wallet10", test_dir),
		client10.clone(),
		Some(seed_phrase),
	);
	wallet_proxy.add_wallet("wallet10", client10.get_send_instance(), wallet10.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!("Wallet Proxy error: {}", e);
		}
	});

	// few values to keep things shorter
	let _reward = core::consensus::REWARD;
	let cm = global::coinbase_maturity() as usize; // assume all testing precedes soft fork height

	// Do some mining
	let mut bh = 20u64;
	let base_amount = consensus::GRIN_BASE;
	let _ = test_framework::award_blocks_to_wallet(&chain, miner.clone(), bh as usize);

	// send some funds to wallets 1
	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet1", base_amount * 1)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet1", base_amount * 2)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet1", base_amount * 3)?;
		bh += 3;
		Ok(())
	})?;

	// 0) Check repair when all is okay should leave wallet contents alone
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.check_repair()?;
		let info = test_framework::wallet_info(api)?;
		assert_eq!(info.amount_currently_spendable, base_amount * 6);
		assert_eq!(info.total, base_amount * 6);
		Ok(())
	})?;

	// send some funds to wallet 2
	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet2", base_amount * 4)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet2", base_amount * 5)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet2", base_amount * 6)?;
		bh += 3;
		Ok(())
	})?;

	let _ = test_framework::award_blocks_to_wallet(&chain, miner.clone(), cm);
	bh += cm as u64;

	// confirm balances
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		assert_eq!(info.amount_currently_spendable, base_amount * 6);
		assert_eq!(info.total, base_amount * 6);
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet2.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		assert_eq!(info.amount_currently_spendable, base_amount * 15);
		assert_eq!(info.total, base_amount * 15);
		Ok(())
	})?;

	// Now there should be outputs on the chain using the same
	// seed + BIP32 path.

	// 1) a full restore should recover all of them:
	wallet::controller::owner_single_use(wallet3.clone(), |api| {
		api.restore()?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet3.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 6);
		assert_eq!(info.amount_currently_spendable, base_amount * 21);
		assert_eq!(info.total, base_amount * 21);
		Ok(())
	})?;

	// 2) check_repair should recover them into a single wallet
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		api.check_repair()?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 6);
		assert_eq!(info.amount_currently_spendable, base_amount * 21);
		Ok(())
	})?;

	// 3) If I recover from seed and start using the wallet without restoring,
	// check_repair should restore the older outputs
	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet4", base_amount * 7)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet4", base_amount * 8)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet4", base_amount * 9)?;
		bh += 3;
		Ok(())
	})?;
	let _ = test_framework::award_blocks_to_wallet(&chain, miner.clone(), cm);
	bh += cm as u64;

	wallet::controller::owner_single_use(wallet4.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 3);
		assert_eq!(info.amount_currently_spendable, base_amount * 24);
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet5.clone(), |api| {
		api.restore()?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet5.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 9);
		assert_eq!(info.amount_currently_spendable, base_amount * (45));
		Ok(())
	})?;

	// 4) If I recover from seed and start using the wallet without restoring,
	// check_repair should restore the older outputs
	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet6", base_amount * 10)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet6", base_amount * 11)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet6", base_amount * 12)?;
		bh += 3;
		Ok(())
	})?;
	let _ = test_framework::award_blocks_to_wallet(&chain, miner.clone(), cm as usize);
	bh += cm as u64;

	wallet::controller::owner_single_use(wallet6.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 3);
		assert_eq!(info.amount_currently_spendable, base_amount * 33);
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet6.clone(), |api| {
		api.check_repair()?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet6.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 12);
		assert_eq!(info.amount_currently_spendable, base_amount * (78));
		Ok(())
	})?;

	// 5) Start using same seed with a different account, amounts should
	// be distinct and restore should return funds from other account

	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet7", base_amount * 13)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet7", base_amount * 14)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet7", base_amount * 15)?;
		bh += 3;
		Ok(())
	})?;

	// mix it up a bit
	wallet::controller::owner_single_use(wallet7.clone(), |api| {
		api.create_account_path("account_1")?;
		api.set_active_account("account_1")?;
		Ok(())
	})?;

	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet7", base_amount * 1)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet7", base_amount * 2)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet7", base_amount * 3)?;
		bh += 3;
		Ok(())
	})?;

	// check balances
	let _ = test_framework::award_blocks_to_wallet(&chain, miner.clone(), cm);
	bh += cm as u64;

	wallet::controller::owner_single_use(wallet7.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 3);
		assert_eq!(info.amount_currently_spendable, base_amount * 6);
		api.set_active_account("default")?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 3);
		assert_eq!(info.amount_currently_spendable, base_amount * 42);
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet8.clone(), |api| {
		api.restore()?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 15);
		assert_eq!(info.amount_currently_spendable, base_amount * 120);
		api.set_active_account("account_1")?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 3);
		assert_eq!(info.amount_currently_spendable, base_amount * 6);
		Ok(())
	})?;

	// 6) Start using same seed with a different account, now overwriting
	// ids on account 2 as well, check_repair should get all outputs created
	// to now into 2 accounts

	wallet::controller::owner_single_use(wallet9.clone(), |api| {
		api.create_account_path("account_1")?;
		api.set_active_account("account_1")?;
		Ok(())
	})?;
	wallet::controller::owner_single_use(miner.clone(), |api| {
		test_framework::send_to_dest(m_client.clone(), api, "wallet9", base_amount * 4)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet9", base_amount * 5)?;
		test_framework::send_to_dest(m_client.clone(), api, "wallet9", base_amount * 6)?;
		bh += 3;
		Ok(())
	})?;

	wallet::controller::owner_single_use(wallet9.clone(), |api| {
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 3);
		assert_eq!(info.amount_currently_spendable, base_amount * 15);
		api.check_repair()?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 6);
		assert_eq!(info.amount_currently_spendable, base_amount * 21);

		api.set_active_account("default")?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 15);
		assert_eq!(info.amount_currently_spendable, base_amount * 120);
		Ok(())
	})?;

	let _ = test_framework::award_blocks_to_wallet(&chain, miner.clone(), cm);
	bh += cm as u64;

	// 7) Ensure check_repair creates missing accounts
	wallet::controller::owner_single_use(wallet10.clone(), |api| {
		api.check_repair()?;
		api.set_active_account("account_1")?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 6);
		assert_eq!(info.amount_currently_spendable, base_amount * 21);

		api.set_active_account("default")?;
		let info = test_framework::wallet_info(api)?;
		let outputs = api.retrieve_outputs(true, false, None)?.1;
		assert_eq!(outputs.len(), 15);
		assert_eq!(info.amount_currently_spendable, base_amount * 120);
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

#[test]
fn two_wallets_one_seed() {
	let test_dir = "test_output/two_wallets_one_seed";
	if let Err(e) = two_wallets_one_seed_impl(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}
