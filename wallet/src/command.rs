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

use crate::util::{Mutex, ZeroingString};
use std::collections::HashMap;
/// Grin wallet command-line function implementations
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json as json;
use uuid::Uuid;

use crate::api::TLSConfig;
use crate::core::core;
use crate::keychain;

use crate::error::{Error, ErrorKind};
use crate::{controller, display, HTTPNodeClient, WalletConfig, WalletInst, WalletSeed};
use crate::{
	FileWalletCommAdapter, HTTPWalletCommAdapter, KeybaseWalletCommAdapter, LMDBBackend,
	NodeClient, NullWalletCommAdapter,
};

/// Arguments common to all wallet commands
#[derive(Clone)]
pub struct GlobalArgs {
	pub account: String,
	pub node_api_secret: Option<String>,
	pub show_spent: bool,
	pub password: Option<ZeroingString>,
	pub tls_conf: Option<TLSConfig>,
}

/// Arguments for init command
pub struct InitArgs {
	/// BIP39 recovery phrase length
	pub list_length: usize,
	pub password: ZeroingString,
	pub config: WalletConfig,
	pub recovery_phrase: Option<ZeroingString>,
	pub restore: bool,
}

pub fn init(g_args: &GlobalArgs, args: InitArgs) -> Result<(), Error> {
	WalletSeed::init_file(
		&args.config,
		args.list_length,
		args.recovery_phrase,
		&args.password,
	)?;
	info!("Wallet seed file created");
	let client_n = HTTPNodeClient::new(
		&args.config.check_node_api_http_addr,
		g_args.node_api_secret.clone(),
	);
	let _: LMDBBackend<HTTPNodeClient, keychain::ExtKeychain> =
		LMDBBackend::new(args.config.clone(), &args.password, client_n)?;
	info!("Wallet database backend created");
	Ok(())
}

/// Argument for recover
pub struct RecoverArgs {
	pub recovery_phrase: Option<ZeroingString>,
	pub passphrase: ZeroingString,
}

/// Check whether seed file exists
pub fn wallet_seed_exists(config: &WalletConfig) -> Result<(), Error> {
	let res = WalletSeed::seed_file_exists(&config)?;
	Ok(res)
}

pub fn recover(config: &WalletConfig, args: RecoverArgs) -> Result<(), Error> {
	if args.recovery_phrase.is_none() {
		let res = WalletSeed::from_file(config, &args.passphrase);
		if let Err(e) = res {
			error!("Error loading wallet seed (check password): {}", e);
			return Err(e);
		}
		let _ = res.unwrap().show_recovery_phrase();
	} else {
		let res = WalletSeed::recover_from_phrase(
			&config,
			&args.recovery_phrase.as_ref().unwrap(),
			&args.passphrase,
		);
		if let Err(e) = res {
			error!("Error recovering seed - {}", e);
			return Err(e);
		}
	}
	Ok(())
}

/// Arguments for listen command
pub struct ListenArgs {
	pub method: String,
}

pub fn listen(config: &WalletConfig, args: &ListenArgs, g_args: &GlobalArgs) -> Result<(), Error> {
	let mut params = HashMap::new();
	params.insert("api_listen_addr".to_owned(), config.api_listen_addr());
	if let Some(t) = g_args.tls_conf.as_ref() {
		params.insert("certificate".to_owned(), t.certificate.clone());
		params.insert("private_key".to_owned(), t.private_key.clone());
	}
	let adapter = match args.method.as_str() {
		"http" => HTTPWalletCommAdapter::new(),
		"keybase" => KeybaseWalletCommAdapter::new(),
		_ => NullWalletCommAdapter::new(),
	};

	let res = adapter.listen(
		params,
		config.clone(),
		&g_args.password.clone().unwrap(),
		&g_args.account,
		g_args.node_api_secret.clone(),
	);
	if let Err(e) = res {
		return Err(ErrorKind::LibWallet(e.kind(), e.cause_string()).into());
	}
	Ok(())
}

pub fn owner_api(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	config: &WalletConfig,
	g_args: &GlobalArgs,
) -> Result<(), Error> {
	let res = controller::owner_listener(
		wallet,
		config.owner_api_listen_addr().as_str(),
		g_args.node_api_secret.clone(),
		g_args.tls_conf.clone(),
		config.owner_api_include_foreign.clone(),
	);
	if let Err(e) = res {
		return Err(ErrorKind::LibWallet(e.kind(), e.cause_string()).into());
	}
	Ok(())
}

/// Arguments for account command
pub struct AccountArgs {
	pub create: Option<String>,
}

pub fn account(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	args: AccountArgs,
) -> Result<(), Error> {
	if args.create.is_none() {
		let res = controller::owner_single_use(wallet, |api| {
			let acct_mappings = api.accounts()?;
			// give logging thread a moment to catch up
			thread::sleep(Duration::from_millis(200));
			display::accounts(acct_mappings);
			Ok(())
		});
		if let Err(e) = res {
			error!("Error listing accounts: {}", e);
			return Err(ErrorKind::LibWallet(e.kind(), e.cause_string()).into());
		}
	} else {
		let label = args.create.unwrap();
		let res = controller::owner_single_use(wallet, |api| {
			api.create_account_path(&label)?;
			thread::sleep(Duration::from_millis(200));
			info!("Account: '{}' Created!", label);
			Ok(())
		});
		if let Err(e) = res {
			thread::sleep(Duration::from_millis(200));
			error!("Error creating account '{}': {}", label, e);
			return Err(ErrorKind::LibWallet(e.kind(), e.cause_string()).into());
		}
	}
	Ok(())
}

/// Arguments for the send command
pub struct SendArgs {
	pub amount: u64,
	pub message: Option<String>,
	pub minimum_confirmations: u64,
	pub selection_strategy: String,
	pub estimate_selection_strategies: bool,
	pub method: String,
	pub dest: String,
	pub change_outputs: usize,
	pub fluff: bool,
	pub max_outputs: usize,
}

pub fn send(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	args: SendArgs,
	dark_scheme: bool,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		if args.estimate_selection_strategies {
			let strategies = vec!["smallest", "all"]
				.into_iter()
				.map(|strategy| {
					let (total, fee) = api
						.estimate_initiate_tx(
							None,
							args.amount,
							args.minimum_confirmations,
							args.max_outputs,
							args.change_outputs,
							strategy == "all",
						)
						.unwrap();
					(strategy, total, fee)
				})
				.collect();
			display::estimate(args.amount, strategies, dark_scheme);
		} else {
			let result = api.initiate_tx(
				None,
				args.amount,
				args.minimum_confirmations,
				args.max_outputs,
				args.change_outputs,
				args.selection_strategy == "all",
				args.message.clone(),
			);
			let (mut slate, lock_fn) = match result {
				Ok(s) => {
					info!(
						"Tx created: {} grin to {} (strategy '{}')",
						core::amount_to_hr_string(args.amount, false),
						args.dest,
						args.selection_strategy,
					);
					s
				}
				Err(e) => {
					info!("Tx not created: {}", e);
					return Err(e);
				}
			};
			let adapter = match args.method.as_str() {
				"http" => HTTPWalletCommAdapter::new(),
				"file" => FileWalletCommAdapter::new(),
				"keybase" => KeybaseWalletCommAdapter::new(),
				"self" => NullWalletCommAdapter::new(),
				_ => NullWalletCommAdapter::new(),
			};
			if adapter.supports_sync() {
				slate = adapter.send_tx_sync(&args.dest, &slate)?;
				api.tx_lock_outputs(&slate, lock_fn)?;
				if args.method == "self" {
					controller::foreign_single_use(wallet, |api| {
						api.receive_tx(&mut slate, Some(&args.dest), None)?;
						Ok(())
					})?;
				}
				if let Err(e) = api.verify_slate_messages(&slate) {
					error!("Error validating participant messages: {}", e);
					return Err(e);
				}
				api.finalize_tx(&mut slate)?;
			} else {
				adapter.send_tx_async(&args.dest, &slate)?;
				api.tx_lock_outputs(&slate, lock_fn)?;
			}
			if adapter.supports_sync() {
				let result = api.post_tx(&slate.tx, args.fluff);
				match result {
					Ok(_) => {
						info!("Tx sent ok",);
						return Ok(());
					}
					Err(e) => {
						error!("Tx sent fail: {}", e);
						return Err(e);
					}
				}
			}
		}
		Ok(())
	})?;
	Ok(())
}

/// Receive command argument
pub struct ReceiveArgs {
	pub input: String,
	pub message: Option<String>,
}

pub fn receive(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	g_args: &GlobalArgs,
	args: ReceiveArgs,
) -> Result<(), Error> {
	let adapter = FileWalletCommAdapter::new();
	let mut slate = adapter.receive_tx_async(&args.input)?;
	controller::foreign_single_use(wallet, |api| {
		if let Err(e) = api.verify_slate_messages(&slate) {
			error!("Error validating participant messages: {}", e);
			return Err(e);
		}
		api.receive_tx(&mut slate, Some(&g_args.account), args.message.clone())?;
		Ok(())
	})?;
	let send_tx = format!("{}.response", args.input);
	adapter.send_tx_async(&send_tx, &slate)?;
	info!(
		"Response file {}.response generated, sending it back to the transaction originator.",
		args.input
	);
	Ok(())
}

/// Finalize command args
pub struct FinalizeArgs {
	pub input: String,
	pub fluff: bool,
}

pub fn finalize(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	args: FinalizeArgs,
) -> Result<(), Error> {
	let adapter = FileWalletCommAdapter::new();
	let mut slate = adapter.receive_tx_async(&args.input)?;
	controller::owner_single_use(wallet.clone(), |api| {
		if let Err(e) = api.verify_slate_messages(&slate) {
			error!("Error validating participant messages: {}", e);
			return Err(e);
		}
		let _ = api.finalize_tx(&mut slate).expect("Finalize failed");

		let result = api.post_tx(&slate.tx, args.fluff);
		match result {
			Ok(_) => {
				info!("Transaction sent successfully, check the wallet again for confirmation.");
				Ok(())
			}
			Err(e) => {
				error!("Tx not sent: {}", e);
				Err(e)
			}
		}
	})?;
	Ok(())
}

/// Info command args
pub struct InfoArgs {
	pub minimum_confirmations: u64,
}

pub fn info(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	g_args: &GlobalArgs,
	args: InfoArgs,
	dark_scheme: bool,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		let (validated, wallet_info) =
			api.retrieve_summary_info(true, args.minimum_confirmations)?;
		display::info(&g_args.account, &wallet_info, validated, dark_scheme);
		Ok(())
	})?;
	Ok(())
}

pub fn outputs(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	g_args: &GlobalArgs,
	dark_scheme: bool,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		let (height, _) = api.node_height()?;
		let (validated, outputs) = api.retrieve_outputs(g_args.show_spent, true, None)?;
		display::outputs(&g_args.account, height, validated, outputs, dark_scheme)?;
		Ok(())
	})?;
	Ok(())
}

/// Txs command args
pub struct TxsArgs {
	pub id: Option<u32>,
}

pub fn txs(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	g_args: &GlobalArgs,
	args: TxsArgs,
	dark_scheme: bool,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		let (height, _) = api.node_height()?;
		let (validated, txs) = api.retrieve_txs(true, args.id, None)?;
		let include_status = !args.id.is_some();
		display::txs(
			&g_args.account,
			height,
			validated,
			&txs,
			include_status,
			dark_scheme,
		)?;
		// if given a particular transaction id, also get and display associated
		// inputs/outputs and messages
		if args.id.is_some() {
			let (_, outputs) = api.retrieve_outputs(true, false, args.id)?;
			display::outputs(&g_args.account, height, validated, outputs, dark_scheme)?;
			// should only be one here, but just in case
			for tx in txs {
				display::tx_messages(&tx, dark_scheme)?;
			}
		};
		Ok(())
	})?;
	Ok(())
}

/// Repost
pub struct RepostArgs {
	pub id: u32,
	pub dump_file: Option<String>,
	pub fluff: bool,
}

pub fn repost(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	args: RepostArgs,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		let (_, txs) = api.retrieve_txs(true, Some(args.id), None)?;
		let stored_tx = api.get_stored_tx(&txs[0])?;
		if stored_tx.is_none() {
			error!(
				"Transaction with id {} does not have transaction data. Not reposting.",
				args.id
			);
			return Ok(());
		}
		match args.dump_file {
			None => {
				if txs[0].confirmed {
					error!(
						"Transaction with id {} is confirmed. Not reposting.",
						args.id
					);
					return Ok(());
				}
				api.post_tx(&stored_tx.unwrap(), args.fluff)?;
				info!("Reposted transaction at {}", args.id);
				return Ok(());
			}
			Some(f) => {
				let mut tx_file = File::create(f.clone())?;
				tx_file.write_all(json::to_string(&stored_tx).unwrap().as_bytes())?;
				tx_file.sync_all()?;
				info!("Dumped transaction data for tx {} to {}", args.id, f);
				return Ok(());
			}
		}
	})?;
	Ok(())
}

/// Cancel
pub struct CancelArgs {
	pub tx_id: Option<u32>,
	pub tx_slate_id: Option<Uuid>,
	pub tx_id_string: String,
}

pub fn cancel(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
	args: CancelArgs,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		let result = api.cancel_tx(args.tx_id, args.tx_slate_id);
		match result {
			Ok(_) => {
				info!("Transaction {} Cancelled", args.tx_id_string);
				Ok(())
			}
			Err(e) => {
				error!("TX Cancellation failed: {}", e);
				Err(e)
			}
		}
	})?;
	Ok(())
}

pub fn restore(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		let result = api.restore();
		match result {
			Ok(_) => {
				warn!("Wallet restore complete",);
				Ok(())
			}
			Err(e) => {
				error!("Wallet restore failed: {}", e);
				error!("Backtrace: {}", e.backtrace().unwrap());
				Err(e)
			}
		}
	})?;
	Ok(())
}

pub fn check_repair(
	wallet: Arc<Mutex<WalletInst<impl NodeClient + 'static, keychain::ExtKeychain>>>,
) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
		warn!("Starting wallet check...",);
		warn!("Updating all wallet outputs, please wait ...",);
		let result = api.check_repair();
		match result {
			Ok(_) => {
				warn!("Wallet check complete",);
				Ok(())
			}
			Err(e) => {
				error!("Wallet check failed: {}", e);
				error!("Backtrace: {}", e.backtrace().unwrap());
				Err(e)
			}
		}
	})?;
	Ok(())
}
