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

//! Provides the JSON/HTTP API for wallets to receive payments. Because
//! receiving money in MimbleWimble requires an interactive exchange, a
//! wallet server that's running at all time is required in many cases.

use std::sync::{Arc, RwLock};

use bodyparser;
use iron::prelude::*;
use iron::status;
use iron::Handler;
use serde_json;

use api;
use core::consensus::reward;
use core::core::{Output, TxKernel};
use core::global;
use failure::{Fail, ResultExt};
use libtx::{reward, slate::Slate};
use libwallet::types::*;
use libwallet::{keys, selection};
use util::LOGGER;

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	pub tx_hex: String,
}

/// Component used to receive coins, implements all the receiving end of the
/// wallet REST API as well as some of the command-line operations.
#[derive(Clone)]
pub struct WalletReceiver<T>
where
	T: WalletBackend,
{
	pub wallet: Arc<RwLock<T>>,
}

impl<T> WalletReceiver<T>
where
	T: WalletBackend,
{
	fn handle_send(&self, wallet: &mut T, slate: &mut Slate) -> Result<(), Error> {
		// create an output using the amount in the slate
		let (_, mut context, receiver_create_fn) =
			selection::build_recipient_output_with_slate(wallet, slate).unwrap();

		// fill public keys
		let _ = slate
			.fill_round_1(
				wallet.keychain(),
				&mut context.sec_key,
				&context.sec_nonce,
				1,
			)
			.context(ErrorKind::LibWalletError)?;

		// perform partial sig
		let _ = slate
			.fill_round_2(wallet.keychain(), &context.sec_key, &context.sec_nonce, 1)
			.context(ErrorKind::LibWalletError)?;

		// Save output in wallet
		let _ = receiver_create_fn(wallet);

		Ok(())
	}
}

impl<T> Handler for WalletReceiver<T>
where
	T: WalletBackend + Send + Sync + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let struct_body = req.get::<bodyparser::Struct<Slate>>();
		let mut wallet = self.wallet.write().unwrap();

		if let Ok(Some(mut slate)) = struct_body {
			let _ = self.handle_send(&mut wallet, &mut slate)
				.map_err(|e| {
					error!(
						LOGGER,
						"Handling send -> Problematic slate, looks like this: {:?}", slate
					);
					e.context(api::ErrorKind::Internal(
						"Error processing partial transaction".to_owned(),
					))
				})
				.unwrap();
			let json = serde_json::to_string(&slate).unwrap();
			Ok(Response::with((status::Ok, json)))
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}

//TODO: Split up the output creation and the wallet insertion
/// Build a coinbase output and the corresponding kernel
pub fn receive_coinbase<T>(
	wallet: &mut T,
	block_fees: &BlockFees,
) -> Result<(Output, TxKernel, BlockFees), Error>
where
	T: WalletBackend,
{
	let root_key_id = wallet.keychain().root_key_id();

	let height = block_fees.height;
	let lock_height = height + global::coinbase_maturity();

	// Now acquire the wallet lock and write the new output.
	let (key_id, derivation) = wallet.with_wallet(|wallet_data| {
		let key_id = block_fees.key_id();
		let (key_id, derivation) = match key_id {
			Some(key_id) => keys::retrieve_existing_key(wallet_data, key_id),
			None => keys::next_available_key(wallet_data),
		};

		// track the new output and return the stuff needed for reward
		wallet_data.add_output(OutputData {
			root_key_id: root_key_id.clone(),
			key_id: key_id.clone(),
			n_child: derivation,
			value: reward(block_fees.fees),
			status: OutputStatus::Unconfirmed,
			height: height,
			lock_height: lock_height,
			is_coinbase: true,
			block: None,
			merkle_proof: None,
		});

		(key_id, derivation)
	})?;

	debug!(
		LOGGER,
		"receive_coinbase: built candidate output - {:?}, {}",
		key_id.clone(),
		derivation,
	);

	let mut block_fees = block_fees.clone();
	block_fees.key_id = Some(key_id.clone());

	debug!(LOGGER, "receive_coinbase: {:?}", block_fees);

	let (out, kern) = reward::output(
		&wallet.keychain(),
		&key_id,
		block_fees.fees,
		block_fees.height,
	).unwrap();
	/* .context(ErrorKind::Keychain)?; */
	Ok((out, kern, block_fees))
}
