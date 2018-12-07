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

//! Test wallet command line works as expected
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate grin_config as config;
extern crate rand;
#[macro_use]
extern crate log;
extern crate chrono;
extern crate serde;
extern crate uuid;
#[macro_use]
extern crate clap;

mod common;
use common::testclient::{LocalWalletClient, WalletProxy};

use std::{fs, env};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use clap::{App, ArgMatches};

use core::global;
use core::global::ChainTypes;
use core::libtx::slate::Slate;
use keychain::ExtKeychain;
use wallet::{libwallet, FileWalletCommAdapter, WalletConfig};
use wallet::command_args;
use config::{GlobalConfig, GlobalWalletConfig};

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

/// Create a wallet config file in the given current directory
pub fn config_command_wallet(dir_name: &str, wallet_name: &str) {
	let mut current_dir;
	let mut default_config = GlobalWalletConfig::default();
		current_dir = env::current_dir().unwrap_or_else(|e| {
		panic!("Error creating config file: {}", e);
	});
	current_dir.push(dir_name);
	current_dir.push(wallet_name);
	let _ = fs::create_dir_all(current_dir.clone());
	let mut config_file_name = current_dir.clone();
	config_file_name.push("grin-wallet.toml");
	if config_file_name.exists() {
		panic!(
			"grin-wallet.toml already exists in the target directory. Please remove it first",
		);
	}
	default_config.update_paths(&current_dir);
	default_config
		.write_to_file(config_file_name.to_str().unwrap())
		.unwrap_or_else(|e| {
			panic!("Error creating config file: {}", e);
		});

	println!(
		"File {} configured and created",
		config_file_name.to_str().unwrap(),
	);
}

/// Handles setup and detection of paths for wallet
pub fn initial_setup_wallet(dir_name: &str, wallet_name: &str) -> WalletConfig {
	let mut current_dir;
	let mut default_config = GlobalWalletConfig::default();
		current_dir = env::current_dir().unwrap_or_else(|e| {
		panic!("Error creating config file: {}", e);
	});
	current_dir.push(dir_name);
	current_dir.push(wallet_name);
	let _ = fs::create_dir_all(current_dir.clone());
	let mut config_file_name = current_dir.clone();
	config_file_name.push("grin-wallet.toml");
	GlobalWalletConfig::new(config_file_name.to_str().unwrap()).unwrap().members.unwrap().wallet
}

fn get_wallet_subcommand<'a>(wallet_dir:&str, wallet_name: &str, args: ArgMatches<'a>) -> ArgMatches<'a> {
	match args.subcommand() {
		("wallet", Some(wallet_args)) => {
			// wallet init command should spit out its config file then continue
			// (if desired)
			if let ("init", Some(init_args)) = wallet_args.subcommand() {
				if init_args.is_present("here") {
					config_command_wallet(wallet_dir, wallet_name);
				}
				init_args.to_owned()
			} else {
				ArgMatches::new()
			}
		}
		_ => {ArgMatches::new()}
	}
}

/// self send impl
fn command_line_test_impl(test_dir: &str) -> Result<(), libwallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);
	let chain = wallet_proxy.chain.clone();
	
	// load app yaml. If it don't exist, just say so and exit
	let yml = load_yaml!("../../src/bin/grin.yml");
	let app = App::from_yaml(yml);

	// wallet init
	let arg_vec = vec!["grin", "wallet", "init", "-h"];
	let args = app.get_matches_from(arg_vec);

	// should create new wallet file
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	let args = get_wallet_subcommand(test_dir, "wallet1", args);
	let config = initial_setup_wallet(test_dir, "wallet1");
	let res = command_args::wallet_command(&args, config, client1);


	// Create us some test wallets
	/*let wallet1 = common::create_wallet(&format!("{}/wallet1", test_dir), client1.clone());
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	let wallet2 = common::create_wallet(&format!("{}/wallet2", test_dir), client2.clone());
	wallet_proxy.add_wallet("wallet2", client2.get_send_instance(), wallet2.clone());

	let client3 = LocalWalletClient::new("wallet3", wallet_proxy.tx.clone());
	let wallet3 = common::create_wallet(&format!("{}/wallet3", test_dir), client3.clone());
	wallet_proxy.add_wallet("wallet3", client3.get_send_instance(), wallet3.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!("Wallet Proxy error: {}", e);
		}
	});

	// few values to keep things shorter
	let reward = core::consensus::REWARD;

	// Get some mining done
	{
		let mut w = wallet1.lock();
		w.set_parent_key_id_by_name("mining")?;
	}
	let mut bh = 10u64;
	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), bh as usize);

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

	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
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

	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
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

	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
	bh += 3;

	// Now repost from cached
	wallet::controller::owner_single_use(wallet1.clone(), |api| {
		let (_, txs) = api.retrieve_txs(true, None, Some(slate.id))?;
		api.post_tx(&txs[0].get_stored_tx().unwrap(), false)?;
		bh += 1;
		Ok(())
	})?;

	let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), 3);
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
	})?;*/

	// let logging finish
	thread::sleep(Duration::from_millis(200));
	Ok(())
}

#[test]
fn wallet_command_line() {
	let test_dir = "test_output/command_line";
	if let Err(e) = command_line_test_impl(test_dir) {
		panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
	}
}
