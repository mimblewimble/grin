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

/// Grin wallet command-line function implementation
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use util::Mutex;

use core::core;
use keychain;

use error::{Error, ErrorKind};
use {WalletConfig, libwallet, display, controller, instantiate_wallet,  WalletInst, HTTPNodeClient};
use {HTTPWalletCommAdapter, FileWalletCommAdapter, NullWalletCommAdapter};

type WalletRef = Arc<Mutex<WalletInst<HTTPNodeClient, keychain::ExtKeychain>>>;

pub fn account(wallet: WalletRef, create:Option<&str>) -> Result<(), Error> {
	if create.is_none() {
		let res = controller::owner_single_use(wallet, |api| {
			let acct_mappings = api.accounts()?;
			// give logging thread a moment to catch up
			thread::sleep(Duration::from_millis(200));
			display::accounts(acct_mappings);
			Ok(())
		});
		if let Err(e) = res {
			error!("Error listing accounts: {}", e);
			std::process::exit(0);
		}
	} else {
		let label = create.unwrap();
		let res = controller::owner_single_use(wallet, |api| {
			api.create_account_path(label)?;
			thread::sleep(Duration::from_millis(200));
			println!("Account: '{}' Created!", label);
			Ok(())
		});
		if let Err(e) = res {
			thread::sleep(Duration::from_millis(200));
			error!("Error creating account '{}': {}", label, e);
			std::process::exit(0);
		}
	}
	Ok(())
}

pub fn send(
	wallet: WalletRef, 
	amount: Option<&str>,
	message: Option<&str>,
	minimum_confirmations: Option<&str>,
	selection_strategy: Option<&str>,
	method: Option<&str>,
	dest: Option<&str>,
	change_outputs: Option<&str>,
	fluff: bool) -> Result<(), Error> {
	let amount = amount.ok_or_else(|| {
		ErrorKind::GenericError("Amount to send required".to_string())
	})?;
	let amount = core::amount_from_hr_string(amount).map_err(|e| {
		ErrorKind::GenericError(format!(
			"Could not parse amount as a number with optional decimal point. e={:?}",
			e
		))
	})?;
	let message = match message {
		Some(m) => Some(m.to_owned()),
		None => None,
	};
	let minimum_confirmations: u64 = minimum_confirmations
		.ok_or_else(|| {
			ErrorKind::GenericError(
				"Minimum confirmations to send required".to_string(),
			)
		}).and_then(|v| {
			v.parse().map_err(|e| {
				ErrorKind::GenericError(format!(
					"Could not parse minimum_confirmations as a whole number. e={:?}",
					e
				))
			})
		})?;
	let selection_strategy =
		selection_strategy.ok_or_else(|| {
			ErrorKind::GenericError("Selection strategy required".to_string())
		})?;
	let method = method.ok_or_else(|| {
		ErrorKind::GenericError("Payment method required".to_string())
	})?;
	let dest = {
		if method == "self" {
			match dest {
				Some(d) => d,
				None => "default",
			}
		} else {
			dest.ok_or_else(|| {
				ErrorKind::GenericError(
					"Destination wallet address required".to_string(),
				)
			})?
		}
	};
	let change_outputs = change_outputs
		.ok_or_else(|| ErrorKind::GenericError("Change outputs required".to_string()))
		.and_then(|v| {
			v.parse().map_err(|e| {
				ErrorKind::GenericError(format!(
					"Failed to parse number of change outputs. e={:?}",
					e
				))
			})
		})?;
	let max_outputs = 500;
	if method == "http" && !dest.starts_with("http://") && !dest.starts_with("https://")
	{
		return Err(ErrorKind::GenericError(format!(
			"HTTP Destination should start with http://: or https://: {}",
			dest
		)).into());
	}
	let res = controller::owner_single_use(wallet.clone(), |api| {
		let result = api.initiate_tx(
			None,
			amount,
			minimum_confirmations,
			max_outputs,
			change_outputs,
			selection_strategy == "all",
			message,
		);
		let (mut slate, lock_fn) = match result {
			Ok(s) => {
				info!(
					"Tx created: {} grin to {} (strategy '{}')",
					core::amount_to_hr_string(amount, false),
					dest,
					selection_strategy,
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
		let adapter = match method {
			"http" => HTTPWalletCommAdapter::new(),
			"file" => FileWalletCommAdapter::new(),
			"self" => NullWalletCommAdapter::new(),
			_ => NullWalletCommAdapter::new(),
		};
		if adapter.supports_sync() {
			slate = adapter.send_tx_sync(dest, &slate)?;
			if method == "self" {
				controller::foreign_single_use(wallet, |api| {
					api.receive_tx(&mut slate, Some(dest), None)?;
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
			adapter.send_tx_async(dest, &slate)?;
			api.tx_lock_outputs(&slate, lock_fn)?;
		}
		if adapter.supports_sync() {
			let result = api.post_tx(&slate.tx, fluff);
			match result {
				Ok(_) => {
					info!("Tx sent",);
					println!("Tx sent",);
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


