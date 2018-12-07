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
extern crate grin_config as config;
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
#[macro_use]
extern crate clap;

mod common;
use common::testclient::{LocalWalletClient, WalletProxy};

use clap::{App, ArgMatches};
use std::thread;
use std::time::Duration;
use std::{env, fs};

use config::GlobalWalletConfig;
use core::global;
use core::global::ChainTypes;
use keychain::ExtKeychain;
use wallet::{command_args, WalletConfig};

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	util::init_test_logger();
	clean_output_dir(test_dir);
	global::set_mining_mode(ChainTypes::AutomatedTesting);
}

/// Create a wallet config file in the given current directory
pub fn config_command_wallet(dir_name: &str, wallet_name: &str) -> Result<(), wallet::Error> {
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
		return Err(wallet::ErrorKind::ArgumentError(
			"grin-wallet.toml already exists in the target directory. Please remove it first"
				.to_owned(),
		))?;
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
	Ok(())
}

/// Handles setup and detection of paths for wallet
pub fn initial_setup_wallet(dir_name: &str, wallet_name: &str) -> WalletConfig {
	let mut current_dir;
	current_dir = env::current_dir().unwrap_or_else(|e| {
		panic!("Error creating config file: {}", e);
	});
	current_dir.push(dir_name);
	current_dir.push(wallet_name);
	let _ = fs::create_dir_all(current_dir.clone());
	let mut config_file_name = current_dir.clone();
	config_file_name.push("grin-wallet.toml");
	GlobalWalletConfig::new(config_file_name.to_str().unwrap())
		.unwrap()
		.members
		.unwrap()
		.wallet
}

fn get_wallet_subcommand<'a>(
	wallet_dir: &str,
	wallet_name: &str,
	args: ArgMatches<'a>,
) -> ArgMatches<'a> {
	match args.subcommand() {
		("wallet", Some(wallet_args)) => {
			// wallet init command should spit out its config file then continue
			// (if desired)
			if let ("init", Some(init_args)) = wallet_args.subcommand() {
				if init_args.is_present("here") {
					let _ = config_command_wallet(wallet_dir, wallet_name);
				}
			}
			wallet_args.to_owned()
		}
		_ => ArgMatches::new(),
	}
}

fn execute_command(
	app: &App,
	test_dir: &str,
	wallet_name: &str,
	client: &LocalWalletClient,
	arg_vec: Vec<&str>,
) -> Result<String, wallet::Error> {
	let args = app.clone().get_matches_from(arg_vec);
	let args = get_wallet_subcommand(test_dir, wallet_name, args.clone());
	let config = initial_setup_wallet(test_dir, wallet_name);
	command_args::wallet_command(&args, config.clone(), client.clone())
}

/// self send impl
fn command_line_test_impl(test_dir: &str) -> Result<(), wallet::Error> {
	setup(test_dir);
	// Create a new proxy to simulate server and wallet responses
	let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> = WalletProxy::new(test_dir);

	// load app yaml. If it don't exist, just say so and exit
	let yml = load_yaml!("../../src/bin/grin.yml");
	let app = App::from_yaml(yml);

	// wallet init
	let arg_vec = vec!["grin", "wallet", "-p", "password", "init", "-h"];
	// should create new wallet file
	let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
	execute_command(&app, test_dir, "wallet1", &client1, arg_vec.clone())?;

	// trying to init twice - should fail
	assert!(execute_command(&app, test_dir, "wallet1", &client1, arg_vec.clone()).is_err());

	// add wallet to proxy
	let wallet1 = common::create_wallet(&format!("{}/wallet1", test_dir), client1.clone());
	wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

	// Create wallet 2
	let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
	execute_command(&app, test_dir, "wallet2", &client2, arg_vec.clone())?;

	let wallet2 = common::create_wallet(&format!("{}/wallet2", test_dir), client2.clone());
	wallet_proxy.add_wallet("wallet2", client2.get_send_instance(), wallet2.clone());

	// Set the wallet proxy listener running
	thread::spawn(move || {
		if let Err(e) = wallet_proxy.run() {
			error!("Wallet Proxy error: {}", e);
		}
	});

	// Create some accounts in wallet 1
	let arg_vec = vec![
		"grin", "wallet", "-p", "password", "account", "-c", "mining",
	];
	execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

	let arg_vec = vec![
		"grin",
		"wallet",
		"-p",
		"password",
		"account",
		"-c",
		"account_1",
	];
	execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

	// Create some accounts in wallet 2
	let arg_vec = vec![
		"grin",
		"wallet",
		"-p",
		"password",
		"account",
		"-c",
		"account_1",
	];
	execute_command(&app, test_dir, "wallet2", &client2, arg_vec.clone())?;
	// already exists
	assert!(execute_command(&app, test_dir, "wallet2", &client2, arg_vec).is_err());

	let arg_vec = vec![
		"grin",
		"wallet",
		"-p",
		"password",
		"account",
		"-c",
		"account_2",
	];
	execute_command(&app, test_dir, "wallet2", &client2, arg_vec)?;

	// let's see those accounts
	let arg_vec = vec!["grin", "wallet", "-p", "password", "account"];
	execute_command(&app, test_dir, "wallet2", &client2, arg_vec)?;

	// Mine a bit into wallet 1 so we have something to send
	//let mut bh = 10u64;
	//let chain = wallet_proxy.chain.clone();
	//let _ = common::award_blocks_to_wallet(&chain, wallet1.clone(), bh as usize);

	// let's see those accounts
	let arg_vec = vec!["grin", "wallet", "-p", "password", "account"];
	execute_command(&app, test_dir, "wallet2", &client2, arg_vec)?;

	// Start wallet 1's listener, collect some coinbase outputs
	let _arg_vec = vec!["grin", "wallet", "-p", "password", "-a", "mining", "listen"];
	//execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

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
