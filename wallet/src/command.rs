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

use std::sync::Arc;
/// Grin wallet command-line function implementation
use std::thread;
use std::time::Duration;
use util::Mutex;

use core::core;
use keychain;

use error::{Error, ErrorKind};
use {controller, display, libwallet, HTTPNodeClient, WalletInst};
use {FileWalletCommAdapter, HTTPWalletCommAdapter, NullWalletCommAdapter};

type WalletRef = Arc<Mutex<WalletInst<HTTPNodeClient, keychain::ExtKeychain>>>;

/// Arguments common to all wallet commands
pub struct GlobalArgs {
	pub account: String,
}

/// Arguments for account command
pub struct AccountArgs {
	pub create: Option<String>,
}

pub fn account(wallet: WalletRef, args: AccountArgs) -> Result<(), Error> {
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
			return Err(ErrorKind::LibWallet(e.kind()).into());
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
			return Err(ErrorKind::LibWallet(e.kind()).into());
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
	pub method: String,
	pub dest: String,
	pub change_outputs: usize,
	pub fluff: bool,
	pub max_outputs: usize,
}

pub fn send(wallet: WalletRef, args: SendArgs) -> Result<(), Error> {
	controller::owner_single_use(wallet.clone(), |api| {
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
				error!("Tx not created: {}", e);
				match e.kind() {
					// user errors, don't backtrace
					libwallet::ErrorKind::NotEnoughFunds { .. } => {}
					libwallet::ErrorKind::FeeDispute { .. } => {}
					libwallet::ErrorKind::FeeExceedsAmount { .. } => {}
					_ => {
						// otherwise give full dump
						error!("Backtrace: {}", e.backtrace().unwrap());
					}
				};
				return Err(e);
			}
		};
		let adapter = match args.method.as_str() {
			"http" => HTTPWalletCommAdapter::new(),
			"file" => FileWalletCommAdapter::new(),
			"self" => NullWalletCommAdapter::new(),
			_ => NullWalletCommAdapter::new(),
		};
		if adapter.supports_sync() {
			slate = adapter.send_tx_sync(&args.dest, &slate)?;
			if args.method == "self" {
				controller::foreign_single_use(wallet, |api| {
					api.receive_tx(&mut slate, Some(&args.dest), None)?;
					Ok(())
				})?;
			}
			api.tx_lock_outputs(&slate, lock_fn)?;
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
					info!("Tx sent",);
					return Ok(());
				}
				Err(e) => {
					error!("Tx not sent: {}", e);
					return Err(e);
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

pub fn receive(wallet: WalletRef, g_args: &GlobalArgs, args: ReceiveArgs) -> Result<(), Error> {
	let adapter = FileWalletCommAdapter::new();
	let mut slate = adapter.receive_tx_async(&args.input)?;
	controller::foreign_single_use(wallet, |api| {
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
