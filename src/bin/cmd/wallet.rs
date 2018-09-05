// Copyright 2018 The Grin Developers
//
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

use std::path::PathBuf;
/// Wallet commands processing
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clap::ArgMatches;

use config::GlobalWalletConfig;
use core::{core, global};
use grin_wallet::{self, controller, display, libwallet};
use grin_wallet::{HTTPWalletClient, LMDBBackend, WalletConfig, WalletInst, WalletSeed};
use keychain;
use servers::start_webwallet_server;
use util::LOGGER;

pub fn _init_wallet_seed(wallet_config: WalletConfig) {
	if let Err(_) = WalletSeed::from_file(&wallet_config) {
		WalletSeed::init_file(&wallet_config).expect("Failed to create wallet seed file.");
	};
}

pub fn seed_exists(wallet_config: WalletConfig) -> bool {
	let mut data_file_dir = PathBuf::new();
	data_file_dir.push(wallet_config.data_file_dir);
	data_file_dir.push(grin_wallet::SEED_FILE);
	if data_file_dir.exists() {
		true
	} else {
		false
	}
}
pub fn instantiate_wallet(
	wallet_config: WalletConfig,
	passphrase: &str,
) -> Box<WalletInst<HTTPWalletClient, keychain::ExtKeychain>> {
	if grin_wallet::needs_migrate(&wallet_config.data_file_dir) {
		// Migrate wallet automatically
		warn!(LOGGER, "Migrating legacy File-Based wallet to LMDB Format");
		if let Err(e) = grin_wallet::migrate(&wallet_config.data_file_dir, passphrase) {
			error!(LOGGER, "Error while trying to migrate wallet: {:?}", e);
			error!(LOGGER, "Please ensure your file wallet files exist and are not corrupted, and that your password is correct");
			panic!();
		} else {
			warn!(LOGGER, "Migration successful. Using LMDB Wallet backend");
		}
		warn!(LOGGER, "Please check the results of the migration process using `grin wallet info` and `grin wallet outputs`");
		warn!(LOGGER, "If anything went wrong, you can try again by deleting the `db` directory and running a wallet command");
		warn!(LOGGER, "If all is okay, you can move/backup/delete all files in the wallet directory EXCEPT FOR wallet.seed");
	}
	let client = HTTPWalletClient::new(&wallet_config.check_node_api_http_addr);
	let db_wallet = LMDBBackend::new(wallet_config.clone(), "", client).unwrap_or_else(|e| {
		panic!(
			"Error creating DB wallet: {} Config: {:?}",
			e, wallet_config
		);
	});
	info!(LOGGER, "Using LMDB Backend for wallet");
	Box::new(db_wallet)
}

pub fn wallet_command(wallet_args: &ArgMatches, config: GlobalWalletConfig) {
	// just get defaults from the global config
	let mut wallet_config = config.members.unwrap().wallet;

	if let Some(t) = wallet_config.chain_type.clone() {
		global::set_mining_mode(t);
	}

	if wallet_args.is_present("external") {
		wallet_config.api_listen_interface = "0.0.0.0".to_string();
	}

	if let Some(dir) = wallet_args.value_of("dir") {
		wallet_config.data_file_dir = dir.to_string().clone();
	}

	if let Some(sa) = wallet_args.value_of("api_server_address") {
		wallet_config.check_node_api_http_addr = sa.to_string().clone();
	}

	let mut show_spent = false;
	if wallet_args.is_present("show_spent") {
		show_spent = true;
	}

	// Derive the keychain based on seed from seed file and specified passphrase.
	// Generate the initial wallet seed if we are running "wallet init".
	if let ("init", Some(_)) = wallet_args.subcommand() {
		WalletSeed::init_file(&wallet_config).expect("Failed to init wallet seed file.");
		info!(LOGGER, "Wallet seed file created");
		let client = HTTPWalletClient::new(&wallet_config.check_node_api_http_addr);
		let _: LMDBBackend<HTTPWalletClient, keychain::ExtKeychain> =
			LMDBBackend::new(wallet_config.clone(), "", client).unwrap_or_else(|e| {
				panic!(
					"Error creating DB for wallet: {} Config: {:?}",
					e, wallet_config
				);
			});
		info!(LOGGER, "Wallet database backend created");
		// give logging thread a moment to catch up
		thread::sleep(Duration::from_millis(200));
		// we are done here with creating the wallet, so just return
		return;
	}

	let passphrase = wallet_args
		.value_of("pass")
		.expect("Failed to read passphrase.");

	// Handle listener startup commands
	{
		let wallet = instantiate_wallet(wallet_config.clone(), passphrase);
		match wallet_args.subcommand() {
			("listen", Some(listen_args)) => {
				if let Some(port) = listen_args.value_of("port") {
					wallet_config.api_listen_port = port.parse().unwrap();
				}
				controller::foreign_listener(wallet, &wallet_config.api_listen_addr())
					.unwrap_or_else(|e| {
						panic!(
							"Error creating wallet listener: {:?} Config: {:?}",
							e, wallet_config
						)
					});
			}
			("owner_api", Some(_api_args)) => {
				controller::owner_listener(wallet, "127.0.0.1:13420").unwrap_or_else(|e| {
					panic!(
						"Error creating wallet api listener: {:?} Config: {:?}",
						e, wallet_config
					)
				});
			}
			("web", Some(_api_args)) => {
				// start owner listener and run static file server
				start_webwallet_server();
				controller::owner_listener(wallet, "127.0.0.1:13420").unwrap_or_else(|e| {
					panic!(
						"Error creating wallet api listener: {:?} Config: {:?}",
						e, wallet_config
					)
				});
			}
			_ => {}
		};
	}

	// Handle single-use (command line) owner commands
	let wallet = Arc::new(Mutex::new(instantiate_wallet(
		wallet_config.clone(),
		passphrase,
	)));
	let res = controller::owner_single_use(wallet, |api| {
		match wallet_args.subcommand() {
			("send", Some(send_args)) => {
				let amount = send_args
					.value_of("amount")
					.expect("Amount to send required");
				let amount = core::amount_from_hr_string(amount)
					.expect("Could not parse amount as a number with optional decimal point.");
				let minimum_confirmations: u64 = send_args
					.value_of("minimum_confirmations")
					.unwrap()
					.parse()
					.expect("Could not parse minimum_confirmations as a whole number.");
				let selection_strategy = send_args
					.value_of("selection_strategy")
					.expect("Selection strategy required");
				let dest = send_args
					.value_of("dest")
					.expect("Destination wallet address required");
				let change_outputs = send_args
					.value_of("change_outputs")
					.unwrap()
					.parse()
					.expect("Failed to parse number of change outputs.");
				let fluff = send_args.is_present("fluff");
				let max_outputs = 500;
				if dest.starts_with("http") {
					let result = api.issue_send_tx(
						amount,
						minimum_confirmations,
						dest,
						max_outputs,
						change_outputs,
						selection_strategy == "all",
					);
					let slate = match result {
						Ok(s) => {
							info!(
								LOGGER,
								"Tx created: {} grin to {} (strategy '{}')",
								core::amount_to_hr_string(amount, false),
								dest,
								selection_strategy,
							);
							s
						}
						Err(e) => {
							error!(LOGGER, "Tx not created: {:?}", e);
							match e.kind() {
								// user errors, don't backtrace
								libwallet::ErrorKind::NotEnoughFunds { .. } => {}
								libwallet::ErrorKind::FeeDispute { .. } => {}
								libwallet::ErrorKind::FeeExceedsAmount { .. } => {}
								_ => {
									// otherwise give full dump
									error!(LOGGER, "Backtrace: {}", e.backtrace().unwrap());
								}
							};
							panic!();
						}
					};
					let result = api.post_tx(&slate, fluff);
					match result {
						Ok(_) => {
							info!(LOGGER, "Tx sent",);
							Ok(())
						}
						Err(e) => {
							error!(LOGGER, "Tx not sent: {:?}", e);
							Err(e)
						}
					}
				} else {
					api.file_send_tx(
						amount,
						minimum_confirmations,
						dest,
						max_outputs,
						change_outputs,
						selection_strategy == "all",
					).expect("Send failed");
					Ok(())
				}
			}
			("receive", Some(send_args)) => {
				let tx_file = send_args
					.value_of("input")
					.expect("Transaction file required");
				api.file_receive_tx(tx_file).expect("Receive failed");
				Ok(())
			}
			("finalize", Some(send_args)) => {
				let fluff = send_args.is_present("fluff");
				let tx_file = send_args
					.value_of("input")
					.expect("Receiver's transaction file required");
				let slate = api.file_finalize_tx(tx_file).expect("Finalize failed");

				let result = api.post_tx(&slate, fluff);
				match result {
					Ok(_) => {
						info!(LOGGER, "Tx sent");
						Ok(())
					}
					Err(e) => {
						error!(LOGGER, "Tx not sent: {:?}", e);
						Err(e)
					}
				}
			}
			("burn", Some(send_args)) => {
				let amount = send_args
					.value_of("amount")
					.expect("Amount to burn required");
				let amount = core::amount_from_hr_string(amount)
					.expect("Could not parse amount as number with optional decimal point.");
				let minimum_confirmations: u64 = send_args
					.value_of("minimum_confirmations")
					.unwrap()
					.parse()
					.expect("Could not parse minimum_confirmations as a whole number.");
				let max_outputs = 500;
				api.issue_burn_tx(amount, minimum_confirmations, max_outputs)
					.unwrap_or_else(|e| {
						panic!("Error burning tx: {:?} Config: {:?}", e, wallet_config)
					});
				Ok(())
			}
			("info", Some(_)) => {
				let (validated, wallet_info) =
					api.retrieve_summary_info(true).unwrap_or_else(|e| {
						panic!(
							"Error getting wallet info: {:?} Config: {:?}",
							e, wallet_config
						)
					});
				display::info(&wallet_info, validated);
				Ok(())
			}
			("outputs", Some(_)) => {
				let (height, _) = api.node_height()?;
				let (validated, outputs) = api.retrieve_outputs(show_spent, true, None)?;
				let _res = display::outputs(height, validated, outputs).unwrap_or_else(|e| {
					panic!(
						"Error getting wallet outputs: {:?} Config: {:?}",
						e, wallet_config
					)
				});
				Ok(())
			}
			("txs", Some(txs_args)) => {
				let tx_id = match txs_args.value_of("id") {
					None => None,
					Some(tx) => match tx.parse() {
						Ok(t) => Some(t),
						Err(_) => panic!("Unable to parse argument 'id' as a number"),
					},
				};
				let (height, _) = api.node_height()?;
				let (validated, txs) = api.retrieve_txs(true, tx_id)?;
				let include_status = !tx_id.is_some();
				let _res =
					display::txs(height, validated, txs, include_status).unwrap_or_else(|e| {
						panic!(
							"Error getting wallet outputs: {} Config: {:?}",
							e, wallet_config
						)
					});
				// if given a particular transaction id, also get and display associated
				// inputs/outputs
				if tx_id.is_some() {
					let (_, outputs) = api.retrieve_outputs(true, false, tx_id)?;
					let _res = display::outputs(height, validated, outputs).unwrap_or_else(|e| {
						panic!(
							"Error getting wallet outputs: {} Config: {:?}",
							e, wallet_config
						)
					});
				};
				Ok(())
			}
			("repost", Some(repost_args)) => {
				let tx_id: u32 = match repost_args.value_of("id") {
					None => {
						error!(LOGGER, "Transaction of a completed but unconfirmed transaction required (specify with --id=[id])");
						panic!();
					}
					Some(tx) => match tx.parse() {
						Ok(t) => t,
						Err(_) => {
							panic!("Unable to parse argument 'id' as a number");
						}
					},
				};
				let dump_file = repost_args.value_of("dumpfile");
				let fluff = repost_args.is_present("fluff");
				match dump_file {
					None => {
						let result = api.post_stored_tx(tx_id, fluff);
						match result {
							Ok(_) => {
								info!(LOGGER, "Reposted transaction at {}", tx_id);
								Ok(())
							}
							Err(e) => {
								error!(LOGGER, "Tranasction reposting failed: {}", e);
								Err(e)
							}
						}
					}
					Some(f) => {
						let result = api.dump_stored_tx(tx_id, f);
						match result {
							Ok(_) => {
								warn!(LOGGER, "Dumped transaction data for tx {} to {}", tx_id, f);
								Ok(())
							}
							Err(e) => {
								error!(LOGGER, "Tranasction reposting failed: {}", e);
								Err(e)
							}
						}
					}
				}
			}
			("cancel", Some(tx_args)) => {
				let tx_id = tx_args
					.value_of("id")
					.expect("'id' argument (-i) is required.");
				let tx_id = tx_id.parse().expect("Could not parse id parameter.");
				let result = api.cancel_tx(tx_id);
				match result {
					Ok(_) => {
						info!(LOGGER, "Transaction {} Cancelled", tx_id);
						Ok(())
					}
					Err(e) => {
						error!(LOGGER, "TX Cancellation failed: {}", e);
						Err(e)
					}
				}
			}
			("restore", Some(_)) => {
				let result = api.restore();
				match result {
					Ok(_) => {
						info!(LOGGER, "Wallet restore complete",);
						Ok(())
					}
					Err(e) => {
						error!(LOGGER, "Wallet restore failed: {:?}", e);
						error!(LOGGER, "Backtrace: {}", e.backtrace().unwrap());
						Err(e)
					}
				}
			}
			_ => panic!("Unknown wallet command, use 'grin help wallet' for details"),
		}
	});
	// we need to give log output a chance to catch up before exiting
	thread::sleep(Duration::from_millis(100));

	if res.is_err() {
		exit(1);
	}
}
