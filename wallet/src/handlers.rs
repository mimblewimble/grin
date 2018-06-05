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

use error::{Error, ErrorKind};
use failure::{Fail, ResultExt};
use libwallet::types::*;
use libwallet::updater;

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
		Ok(updater::build_coinbase(wallet, block_fees).context(ErrorKind::Node)?)
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
