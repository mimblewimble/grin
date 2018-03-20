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

use futures::{Future, Stream};

use hyper::Error as HyperError;
use hyper::server::{Request, Response};

use serde_json;

use receiver::receive_coinbase;
use core::ser;
use api;
use api::rest::{error_response, Handler, json_response, PathParams};
use keychain::Keychain;
use types::*;
use util;
use failure::{Fail, ResultExt};

pub struct CoinbaseHandler {
	pub config: WalletConfig,
	pub keychain: Keychain,
}

impl CoinbaseHandler {
	fn build_coinbase(&self, block_fees: &BlockFees) -> Result<CbData, Error> {
		let (out, kern, block_fees) = receive_coinbase(&self.config, &self.keychain, block_fees)
			.map_err(|e| api::Error::Internal(format!("Error building coinbase: {:?}", e)))
			.context(ErrorKind::Node)?;

		let out_bin = ser::ser_vec(&out)
			.map_err(|e| api::Error::Internal(format!("Error serializing output: {:?}", e)))
			.context(ErrorKind::Node)?;

		let kern_bin = ser::ser_vec(&kern)
			.map_err(|e| api::Error::Internal(format!("Error serializing kernel: {:?}", e)))
			.context(ErrorKind::Node)?;

		let key_id_bin = match block_fees.key_id {
			Some(key_id) => ser::ser_vec(&key_id)
				.map_err(|e| api::Error::Internal(format!("Error serializing kernel: {:?}", e)))
				.context(ErrorKind::Node)?,
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
impl Handler for CoinbaseHandler {
	fn handle(&self, req: Request, _params: PathParams) -> Result<Response, HyperError> {
		let _ = req.body().concat2().and_then(move |body| {
			let block_fees: BlockFees = match serde_json::from_slice(&body) {
				Ok(block_fees) => block_fees,
				Err(e) => return error_response(api::Error::Argument(e.to_string())),
			};

			// TODO - refactoring the commented line below upon new error mapping of Fail between client/server.
			// commenting out the following line until setting standard of 
			// how to map Fail errors into Http StatusCode for client/server error handling/mapping.
			// reference) https://github.com/mimblewimble/grin/pull/713
			// Currently using mapping between Api::Error and Hyper::StatusCode.
			/*
			let coinbase = self.build_coinbase(&block_fees)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
			*/
			let coinbase = match self.build_coinbase(&block_fees) {
				Ok(coinbase) => coinbase,
				Err(e) => return error_response(api::Error::Argument(e.to_string())),
			};
			return json_response(&coinbase);
		});
		Ok(Response::new())
	}
}
