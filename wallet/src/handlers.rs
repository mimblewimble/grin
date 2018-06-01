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
use std::sync::{Arc, RwLock};

use bodyparser;
use iron::Handler;
use iron::prelude::*;
use iron::status;
use serde_json;

use core::ser;
use error::{Error, ErrorKind};
use failure::{Fail, ResultExt};
use libwallet::types::*;
use receiver::receive_coinbase;
use util;

pub struct CoinbaseHandler<T>
where
	T: WalletBackend,
{
	pub wallet: Arc<RwLock<T>>,
}

impl<T> CoinbaseHandler<T>
where
	T: WalletBackend,
{
	fn build_coinbase(&self, wallet: &mut T, block_fees: &BlockFees) -> Result<CbData, Error> {
		let (out, kern, block_fees) =
			receive_coinbase(wallet, block_fees).context(ErrorKind::Node)?;

		let out_bin = ser::ser_vec(&out).context(ErrorKind::Node)?;

		let kern_bin = ser::ser_vec(&kern).context(ErrorKind::Node)?;

		let key_id_bin = match block_fees.key_id {
			Some(key_id) => ser::ser_vec(&key_id).context(ErrorKind::Node)?,
			None => vec![],
		};

		Ok(CbData {
			output: util::to_hex(out_bin),
			kernel: util::to_hex(kern_bin),
			key_id: util::to_hex(key_id_bin),
		})
	}
}

// TODO - error handling - what to return if we fail to get the wallet lock for
// some reason...
impl<T> Handler for CoinbaseHandler<T>
where
	T: WalletBackend + Send + Sync + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let struct_body = req.get::<bodyparser::Struct<BlockFees>>();
		let mut wallet = self.wallet.write().unwrap();
		if let Ok(Some(block_fees)) = struct_body {
			let coinbase = self.build_coinbase(&mut wallet, &block_fees)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
			if let Ok(json) = serde_json::to_string(&coinbase) {
				Ok(Response::with((status::Ok, json)))
			} else {
				Ok(Response::with((status::BadRequest, "")))
			}
		} else {
			Ok(Response::with((status::BadRequest, "")))
		}
	}
}
