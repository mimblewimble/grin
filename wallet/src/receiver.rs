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
use iron::Handler;
use iron::prelude::*;
use iron::status;
use serde_json;

use api;
use error::Error;
use failure::Fail;
use libtx::slate::Slate;
use libwallet::selection;
use libwallet::types::*;
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
		let _ = slate.fill_round_1(
			wallet.keychain(),
			&mut context.sec_key,
			&context.sec_nonce,
			1,
		)?;

		// perform partial sig
		let _ = slate.fill_round_2(wallet.keychain(), &context.sec_key, &context.sec_nonce, 1)?;

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
