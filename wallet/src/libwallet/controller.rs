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

//! Controller for wallet.. instantiates and handles listeners (or single-run
//! invocations) as needed.
//! Still experimental
use api::ApiServer;
use std::sync::{Arc, Mutex};

use bodyparser;
use iron::Handler;
use iron::prelude::*;
use iron::status;
use serde::Serialize;
use serde_json;

use failure::Fail;

use libtx::slate::Slate;
use libwallet::api::{APIForeign, APIOwner};
use libwallet::types::{BlockFees, CbData, OutputData, WalletBackend, WalletInfo};
use libwallet::{Error, ErrorKind};

use util::LOGGER;

/// Instantiate wallet Owner API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn owner_single_use<F, T>(wallet: &mut T, f: F) -> Result<(), Error>
where
	T: WalletBackend,
	F: FnOnce(&mut APIOwner<T>) -> Result<(), Error>,
{
	wallet.open_with_credentials()?;
	f(&mut APIOwner::new(wallet))?;
	wallet.close()?;
	Ok(())
}

/// Instantiate wallet Foreign API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn foreign_single_use<F, T>(wallet: &mut T, f: F) -> Result<(), Error>
where
	T: WalletBackend,
	F: FnOnce(&mut APIForeign<T>) -> Result<(), Error>,
{
	wallet.open_with_credentials()?;
	f(&mut APIForeign::new(wallet))?;
	wallet.close()?;
	Ok(())
}

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn owner_listener<T>(wallet: T, addr: &str) -> Result<(), Error>
where
	T: WalletBackend,
	OwnerAPIHandler<T>: Handler,
{
	let api_handler = OwnerAPIHandler {
		wallet: Arc::new(Mutex::new(wallet)),
	};

	let router = router!(
		receive_tx: get "/wallet/owner/*" => api_handler,
	);

	let mut apis = ApiServer::new("/v1".to_string());
	apis.register_handler(router);
	match apis.start(addr) {
		Err(e) => error!(
			LOGGER,
			"Failed to start Grin wallet owner API listener: {}.", e
		),
		Ok(_) => info!(LOGGER, "Grin wallet owner API listener started at {}", addr),
	};
	Ok(())
}

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn foreign_listener<T>(wallet: T, addr: &str) -> Result<(), Error>
where
	T: WalletBackend,
	ForeignAPIHandler<T>: Handler,
{
	let api_handler = ForeignAPIHandler {
		wallet: Arc::new(Mutex::new(wallet)),
	};

	let router = router!(
		receive_tx: post "/wallet/foreign/*" => api_handler,
	);

	let mut apis = ApiServer::new("/v1".to_string());
	apis.register_handler(router);
	match apis.start(addr) {
		Err(e) => error!(
			LOGGER,
			"Failed to start Grin wallet foreign listener: {}.", e
		),
		Ok(_) => info!(LOGGER, "Grin wallet foreign listener started at {}", addr),
	};
	Ok(())
}
/// API Handler/Wrapper for owner functions

pub struct OwnerAPIHandler<T>
where
	T: WalletBackend,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<T>>,
}

impl<T> OwnerAPIHandler<T>
where
	T: WalletBackend,
{
	fn retrieve_outputs(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T>,
	) -> Result<Vec<OutputData>, Error> {
		let res = api.retrieve_outputs(false)?;
		Ok(res.1)
	}

	fn retrieve_summary_info(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T>,
	) -> Result<WalletInfo, Error> {
		let res = api.retrieve_summary_info()?;
		Ok(res.1)
	}

	fn issue_send_tx(&self, req: &mut Request, api: &mut APIOwner<T>) -> Result<(), Error> {
		// TODO: Args
		api.issue_send_tx(60, 10, "", 1000, true, true)
	}

	fn issue_burn_tx(&self, req: &mut Request, api: &mut APIOwner<T>) -> Result<(), Error> {
		// TODO: Args
		api.issue_burn_tx(60, 10, 1000)
	}

	fn handle_request(&self, req: &mut Request, api: &mut APIOwner<T>) -> IronResult<Response> {
		let url = req.url.clone();
		let path_elems = url.path();
		match *path_elems.last().unwrap() {
			"retrieve_outputs" => json_response_pretty(&self.retrieve_outputs(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			"retrieve_summary_info" => json_response_pretty(&self.retrieve_summary_info(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			"issue_send_tx" => json_response_pretty(&self.issue_send_tx(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			"issue_burn_tx" => json_response_pretty(&self.issue_burn_tx(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			_ => Err(IronError::new(
				Fail::compat(ErrorKind::Hyper),
				status::BadRequest,
			)),
		}
	}
}

impl<T> Handler for OwnerAPIHandler<T>
where
	T: WalletBackend + Send + Sync + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		// every request should open with stored credentials,
		// do its thing and then de-init whatever secrets have been
		// stored
		let mut wallet = self.wallet.lock().unwrap();
		wallet.open_with_credentials().map_err(|e| {
			error!(LOGGER, "Error opening wallet: {:?}", e);
			IronError::new(Fail::compat(e), status::BadRequest)
		})?;
		let mut api = APIOwner::new(&mut *wallet);
		let resp_json = self.handle_request(req, &mut api);
		api.wallet
			.close()
			.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
		resp_json
	}
}

/// API Handler/Wrapper for foreign functions

pub struct ForeignAPIHandler<T>
where
	T: WalletBackend,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<T>>,
}

impl<T> ForeignAPIHandler<T>
where
	T: WalletBackend,
{
	fn build_coinbase(&self, req: &mut Request, api: &mut APIForeign<T>) -> Result<CbData, Error> {
		let struct_body = req.get::<bodyparser::Struct<BlockFees>>();
		match struct_body {
			Ok(Some(block_fees)) => api.build_coinbase(&block_fees),
			Ok(None) => {
				error!(LOGGER, "Missing request body: build_coinbase");
				Err(ErrorKind::GenericError(
					"Invalid request body: build_coinbase",
				))?
			}
			Err(e) => {
				error!(LOGGER, "Invalid request body: build_coinbase: {:?}", e);
				Err(ErrorKind::GenericError(
					"Invalid request body: build_coinbase",
				))?
			}
		}
	}

	fn receive_tx(&self, req: &mut Request, api: &mut APIForeign<T>) -> Result<Slate, Error> {
		let struct_body = req.get::<bodyparser::Struct<Slate>>();
		if let Ok(Some(mut slate)) = struct_body {
			api.receive_tx(&mut slate)?;
			Ok(slate.clone())
		} else {
			Err(ErrorKind::GenericError("Invalid request body: receive_tx"))?
		}
	}

	fn handle_request(&self, req: &mut Request, api: &mut APIForeign<T>) -> IronResult<Response> {
		let url = req.url.clone();
		let path_elems = url.path();
		match *path_elems.last().unwrap() {
			"build_coinbase" => json_response(&self.build_coinbase(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			"receive_tx" => json_response(&self.receive_tx(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			_ => Err(IronError::new(
				Fail::compat(ErrorKind::Hyper),
				status::BadRequest,
			)),
		}
	}
}

impl<T> Handler for ForeignAPIHandler<T>
where
	T: WalletBackend + Send + Sync + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		// every request should open with stored credentials,
		// do its thing and then de-init whatever secrets have been
		// stored
		let mut wallet = self.wallet.lock().unwrap();
		wallet.open_with_credentials().map_err(|e| {
			error!(LOGGER, "Error opening wallet: {:?}", e);
			IronError::new(Fail::compat(e), status::BadRequest)
		})?;
		let mut api = APIForeign::new(&mut *wallet);
		let resp_json = self.handle_request(req, &mut api);
		api.wallet
			.close()
			.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?;
		resp_json
	}
}

// Utility to serialize a struct into JSON and produce a sensible IronResult
// out of it.
fn json_response<T>(s: &T) -> IronResult<Response>
where
	T: Serialize,
{
	match serde_json::to_string(s) {
		Ok(json) => Ok(Response::with((status::Ok, json))),
		Err(_) => Ok(Response::with((status::InternalServerError, ""))),
	}
}

// pretty-printed version of above
fn json_response_pretty<T>(s: &T) -> IronResult<Response>
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => Ok(Response::with((status::Ok, json))),
		Err(_) => Ok(Response::with((status::InternalServerError, ""))),
	}
}
