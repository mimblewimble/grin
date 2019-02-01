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
#[cfg(test)]
mod wallet_tests {
	use clap;
	use grin_util as util;
	use grin_wallet;

	use grin_wallet::test_framework::{self, LocalWalletClient, WalletProxy};

	use clap::{App, ArgMatches};
	use grin_util::Mutex;
	use std::sync::Arc;
	use std::thread;
	use std::time::Duration;
	use std::{env, fs};

	use grin_config::GlobalWalletConfig;
	use grin_core::global;
	use grin_core::global::ChainTypes;
	use grin_keychain::ExtKeychain;
	use grin_wallet::{LMDBBackend, WalletBackend, WalletConfig, WalletInst, WalletSeed};

	use super::super::wallet_args;

	fn clean_output_dir(test_dir: &str) {
		let _ = fs::remove_dir_all(test_dir);
	}

	fn setup(test_dir: &str) {
		util::init_test_logger();
		clean_output_dir(test_dir);
		global::set_mining_mode(ChainTypes::AutomatedTesting);
	}

	/// Create a wallet config file in the given current directory
	pub fn config_command_wallet(
		dir_name: &str,
		wallet_name: &str,
	) -> Result<(), grin_wallet::Error> {
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
			return Err(grin_wallet::ErrorKind::ArgumentError(
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
	//
	// Helper to create an instance of the LMDB wallet
	fn instantiate_wallet(
		mut wallet_config: WalletConfig,
		node_client: LocalWalletClient,
		passphrase: &str,
		account: &str,
	) -> Result<Arc<Mutex<WalletInst<LocalWalletClient, ExtKeychain>>>, grin_wallet::Error> {
		wallet_config.chain_type = None;
		// First test decryption, so we can abort early if we have the wrong password
		let _ = WalletSeed::from_file(&wallet_config, passphrase)?;
		let mut db_wallet = LMDBBackend::new(wallet_config.clone(), passphrase, node_client)?;
		db_wallet.set_parent_key_id_by_name(account)?;
		info!("Using LMDB Backend for wallet");
		Ok(Arc::new(Mutex::new(db_wallet)))
	}

	fn execute_command(
		app: &App,
		test_dir: &str,
		wallet_name: &str,
		client: &LocalWalletClient,
		arg_vec: Vec<&str>,
	) -> Result<String, grin_wallet::Error> {
		let args = app.clone().get_matches_from(arg_vec);
		let args = get_wallet_subcommand(test_dir, wallet_name, args.clone());
		let mut config = initial_setup_wallet(test_dir, wallet_name);
		//unset chain type so it doesn't get reset
		config.chain_type = None;
		wallet_args::wallet_command(&args, config.clone(), client.clone())
	}

	/// command line tests
	fn command_line_test_impl(test_dir: &str) -> Result<(), grin_wallet::Error> {
		setup(test_dir);
		// Create a new proxy to simulate server and wallet responses
		let mut wallet_proxy: WalletProxy<LocalWalletClient, ExtKeychain> =
			WalletProxy::new(test_dir);
		let chain = wallet_proxy.chain.clone();

		// load app yaml. If it don't exist, just say so and exit
		let yml = load_yaml!("../grin.yml");
		let app = App::from_yaml(yml);

		// wallet init
		let arg_vec = vec!["grin", "wallet", "-p", "password", "init", "-h"];
		// should create new wallet file
		let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec.clone())?;

		// trying to init twice - should fail
		assert!(execute_command(&app, test_dir, "wallet1", &client1, arg_vec.clone()).is_err());
		let client1 = LocalWalletClient::new("wallet1", wallet_proxy.tx.clone());

		// add wallet to proxy
		//let wallet1 = test_framework::create_wallet(&format!("{}/wallet1", test_dir), client1.clone());
		let config1 = initial_setup_wallet(test_dir, "wallet1");
		let wallet1 = instantiate_wallet(config1.clone(), client1.clone(), "password", "default")?;
		wallet_proxy.add_wallet("wallet1", client1.get_send_instance(), wallet1.clone());

		// Create wallet 2
		let client2 = LocalWalletClient::new("wallet2", wallet_proxy.tx.clone());
		execute_command(&app, test_dir, "wallet2", &client2, arg_vec.clone())?;

		let config2 = initial_setup_wallet(test_dir, "wallet2");
		let wallet2 = instantiate_wallet(config2.clone(), client2.clone(), "password", "default")?;
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

		// let's see those accounts
		let arg_vec = vec!["grin", "wallet", "-p", "password", "account"];
		execute_command(&app, test_dir, "wallet2", &client2, arg_vec)?;

		// Mine a bit into wallet 1 so we have something to send
		// (TODO: Be able to stop listeners so we can test this better)
		let wallet1 = instantiate_wallet(config1.clone(), client1.clone(), "password", "default")?;
		grin_wallet::controller::owner_single_use(wallet1.clone(), |api| {
			api.set_active_account("mining")?;
			Ok(())
		})?;

		let mut bh = 10u64;
		let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), bh as usize);

		let very_long_message = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef\
		                         This part should all be truncated";

		// Update info and check
		let arg_vec = vec!["grin", "wallet", "-p", "password", "-a", "mining", "info"];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		// try a file exchange
		let file_name = format!("{}/tx1.part_tx", test_dir);
		let response_file_name = format!("{}/tx1.part_tx.response", test_dir);
		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"mining",
			"send",
			"-m",
			"file",
			"-d",
			&file_name,
			"-g",
			very_long_message,
			"10",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"account_1",
			"receive",
			"-i",
			&file_name,
			"-g",
			"Thanks, Yeast!",
		];
		execute_command(&app, test_dir, "wallet2", &client2, arg_vec.clone())?;

		// shouldn't be allowed to receive twice
		assert!(execute_command(&app, test_dir, "wallet2", &client2, arg_vec).is_err());

		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"finalize",
			"-i",
			&response_file_name,
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;
		bh += 1;

		let wallet1 = instantiate_wallet(config1.clone(), client1.clone(), "password", "default")?;

		// Check our transaction log, should have 10 entries
		grin_wallet::controller::owner_single_use(wallet1.clone(), |api| {
			api.set_active_account("mining")?;
			let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
			assert!(refreshed);
			assert_eq!(txs.len(), bh as usize);
			Ok(())
		})?;

		let _ = test_framework::award_blocks_to_wallet(&chain, wallet1.clone(), 10);
		bh += 10;

		// update info for each
		let arg_vec = vec!["grin", "wallet", "-p", "password", "-a", "mining", "info"];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"account_1",
			"info",
		];
		execute_command(&app, test_dir, "wallet2", &client1, arg_vec)?;

		// check results in wallet 2
		let wallet2 = instantiate_wallet(config2.clone(), client2.clone(), "password", "default")?;
		grin_wallet::controller::owner_single_use(wallet2.clone(), |api| {
			api.set_active_account("account_1")?;
			let (_, wallet1_info) = api.retrieve_summary_info(true, 1)?;
			assert_eq!(wallet1_info.last_confirmed_height, bh);
			assert_eq!(wallet1_info.amount_currently_spendable, 10_000_000_000);
			Ok(())
		})?;

		// Self-send to same account, using smallest strategy
		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"mining",
			"send",
			"-m",
			"file",
			"-d",
			&file_name,
			"-g",
			"Love, Yeast, Smallest",
			"-s",
			"smallest",
			"10",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"mining",
			"receive",
			"-i",
			&file_name,
			"-g",
			"Thanks, Yeast!",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec.clone())?;

		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"finalize",
			"-i",
			&response_file_name,
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;
		bh += 1;

		// Check our transaction log, should have bh entries + one for the self receive
		let wallet1 = instantiate_wallet(config1.clone(), client1.clone(), "password", "default")?;

		grin_wallet::controller::owner_single_use(wallet1.clone(), |api| {
			api.set_active_account("mining")?;
			let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
			assert!(refreshed);
			assert_eq!(txs.len(), bh as usize + 1);
			Ok(())
		})?;

		// Try using the self-send method, splitting up outputs for the fun of it
		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"mining",
			"send",
			"-m",
			"self",
			"-d",
			"mining",
			"-g",
			"Self love",
			"-o",
			"3",
			"-s",
			"smallest",
			"10",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;
		bh += 1;

		// Check our transaction log, should have bh entries + 2 for the self receives
		let wallet1 = instantiate_wallet(config1.clone(), client1.clone(), "password", "default")?;

		grin_wallet::controller::owner_single_use(wallet1.clone(), |api| {
			api.set_active_account("mining")?;
			let (refreshed, txs) = api.retrieve_txs(true, None, None)?;
			assert!(refreshed);
			assert_eq!(txs.len(), bh as usize + 2);
			Ok(())
		})?;

		// Another file exchange, don't send, but unlock with repair command
		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"mining",
			"send",
			"-m",
			"file",
			"-d",
			&file_name,
			"-g",
			"Ain't sending",
			"10",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		let arg_vec = vec!["grin", "wallet", "-p", "password", "check"];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		// Another file exchange, cancel this time
		let arg_vec = vec![
			"grin",
			"wallet",
			"-p",
			"password",
			"-a",
			"mining",
			"send",
			"-m",
			"file",
			"-d",
			&file_name,
			"-g",
			"Ain't sending 2",
			"10",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		let arg_vec = vec![
			"grin", "wallet", "-p", "password", "-a", "mining", "cancel", "-i", "26",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		// txs and outputs (mostly spit out for a visual in test logs)
		let arg_vec = vec!["grin", "wallet", "-p", "password", "-a", "mining", "txs"];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		// message output (mostly spit out for a visual in test logs)
		let arg_vec = vec![
			"grin", "wallet", "-p", "password", "-a", "mining", "txs", "-i", "10",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		// txs and outputs (mostly spit out for a visual in test logs)
		let arg_vec = vec![
			"grin", "wallet", "-p", "password", "-a", "mining", "outputs",
		];
		execute_command(&app, test_dir, "wallet1", &client1, arg_vec)?;

		// let logging finish
		thread::sleep(Duration::from_millis(200));
		Ok(())
	}

	#[test]
	fn wallet_command_line() {
		let test_dir = "target/test_output/command_line";
		if let Err(e) = command_line_test_impl(test_dir) {
			panic!("Libwallet Error: {} - {}", e, e.backtrace().unwrap());
		}
	}
}
