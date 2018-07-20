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
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use bodyparser;
use iron::prelude::{IronError, IronResult, Plugin, Request, Response};
use iron::{status, Handler, Headers};
use serde::Serialize;
use serde_json;
use urlencoded::UrlEncodedQuery;

use failure::Fail;

use keychain::Keychain;
use libtx::slate::Slate;
use libwallet::api::{APIForeign, APIOwner};
use libwallet::types::{
	BlockFees, CbData, OutputData, SendTXArgs, WalletBackend, WalletClient, WalletInfo,
};
use libwallet::{Error, ErrorKind};

use util::LOGGER;

/// Instantiate wallet Owner API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn owner_single_use<F, T: ?Sized, C, K>(wallet: Arc<Mutex<Box<T>>>, f: F) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	F: FnOnce(&mut APIOwner<T, C, K>) -> Result<(), Error>,
	C: WalletClient,
	K: Keychain,
{
	f(&mut APIOwner::new(wallet.clone()))?;
	Ok(())
}

/// Instantiate wallet Foreign API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn foreign_single_use<F, T: ?Sized, C, K>(wallet: Arc<Mutex<Box<T>>>, f: F) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	F: FnOnce(&mut APIForeign<T, C, K>) -> Result<(), Error>,
	C: WalletClient,
	K: Keychain,
{
	f(&mut APIForeign::new(wallet.clone()))?;
	Ok(())
}

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn owner_listener<T: ?Sized, C, K>(wallet: Box<T>, addr: &str) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	OwnerAPIGetHandler<T, C, K>: Handler,
	OwnerAPIPostHandler<T, C, K>: Handler,
	C: WalletClient,
	K: Keychain,
{
	let wallet_arc = Arc::new(Mutex::new(wallet));
	let api_get_handler = OwnerAPIGetHandler::new(wallet_arc.clone());
	let api_post_handler = OwnerAPIPostHandler::new(wallet_arc);
	let api_options_handler = OwnerAPIOptionsHandler {};

	let router = router!(
		owner_options: options "/wallet/owner/*" => api_options_handler,
		owner_get: get "/wallet/owner/*" => api_get_handler,
		owner_post: post "/wallet/owner/*" => api_post_handler,
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
pub fn foreign_listener<T: ?Sized, C, K>(wallet: Box<T>, addr: &str) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	ForeignAPIHandler<T, C, K>: Handler,
	C: WalletClient,
	K: Keychain,
{
	let api_handler = ForeignAPIHandler::new(Arc::new(Mutex::new(wallet)));

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

pub struct OwnerAPIGetHandler<T: ?Sized, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<Box<T>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> OwnerAPIGetHandler<T, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Create a new owner API handler for GET methods
	pub fn new(wallet: Arc<Mutex<Box<T>>>) -> OwnerAPIGetHandler<T, C, K> {
		OwnerAPIGetHandler {
			wallet,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	fn retrieve_outputs(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T, C, K>,
	) -> Result<(bool, Vec<OutputData>), Error> {
		let mut update_from_node = false;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(_) = params.get("refresh") {
				update_from_node = true;
			}
		}
		api.retrieve_outputs(false, update_from_node, None)
	}

	fn retrieve_summary_info(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T, C, K>,
	) -> Result<(bool, WalletInfo), Error> {
		let mut update_from_node = false;
		if let Ok(params) = req.get_ref::<UrlEncodedQuery>() {
			if let Some(_) = params.get("refresh") {
				update_from_node = true;
			}
		}
		api.retrieve_summary_info(update_from_node)
	}

	fn node_height(
		&self,
		_req: &mut Request,
		api: &mut APIOwner<T, C, K>,
	) -> Result<(u64, bool), Error> {
		api.node_height()
	}

	fn handle_request(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T, C, K>,
	) -> IronResult<Response> {
		let url = req.url.clone();
		let path_elems = url.path();
		match *path_elems.last().unwrap() {
			"retrieve_outputs" => json_response(&self.retrieve_outputs(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			"retrieve_summary_info" => json_response(&self.retrieve_summary_info(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			"node_height" => json_response(&self.node_height(req, api)
				.map_err(|e| IronError::new(Fail::compat(e), status::BadRequest))?),
			_ => Err(IronError::new(
				Fail::compat(ErrorKind::Hyper),
				status::BadRequest,
			)),
		}
	}
}

impl<T: ?Sized, C, K> Handler for OwnerAPIGetHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let mut api = APIOwner::new(self.wallet.clone());
		let mut resp_json = self.handle_request(req, &mut api);
		if !resp_json.is_err() {
			resp_json
				.as_mut()
				.unwrap()
				.headers
				.set_raw("access-control-allow-origin", vec![b"*".to_vec()]);
		}
		resp_json
	}
}

/// Handles all owner API POST requests
pub struct OwnerAPIPostHandler<T: ?Sized, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<Box<T>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> OwnerAPIPostHandler<T, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// New POST handler
	pub fn new(wallet: Arc<Mutex<Box<T>>>) -> OwnerAPIPostHandler<T, C, K> {
		OwnerAPIPostHandler {
			wallet,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	fn issue_send_tx(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T, C, K>,
	) -> Result<Slate, Error> {
		let struct_body = req.get::<bodyparser::Struct<SendTXArgs>>();
		match struct_body {
			Ok(Some(args)) => api.issue_send_tx(
				args.amount,
				args.minimum_confirmations,
				&args.dest,
				args.max_outputs,
				args.selection_strategy_is_use_all,
			),
			Ok(None) => {
				error!(LOGGER, "Missing request body: issue_send_tx");
				Err(ErrorKind::GenericError(
					"Invalid request body: issue_send_tx".to_owned(),
				))?
			}
			Err(e) => {
				error!(LOGGER, "Invalid request body: issue_send_tx {:?}", e);
				Err(ErrorKind::GenericError(
					"Invalid request body: issue_send_tx".to_owned(),
				))?
			}
		}
	}

	fn issue_burn_tx(&self, _req: &mut Request, api: &mut APIOwner<T, C, K>) -> Result<(), Error> {
		// TODO: Args
		api.issue_burn_tx(60, 10, 1000)
	}

	fn handle_request(
		&self,
		req: &mut Request,
		api: &mut APIOwner<T, C, K>,
	) -> Result<String, Error> {
		let url = req.url.clone();
		let path_elems = url.path();
		match *path_elems.last().unwrap() {
			"issue_send_tx" => json_response_pretty(&self.issue_send_tx(req, api)?),
			"issue_burn_tx" => json_response_pretty(&self.issue_burn_tx(req, api)?),
			_ => Err(ErrorKind::GenericError(
				"Unknown error handling post request".to_owned(),
			))?,
		}
	}

	fn create_error_response(&self, e: Error) -> IronResult<Response> {
		let mut headers = Headers::new();
		headers.set_raw("access-control-allow-origin", vec![b"*".to_vec()]);
		headers.set_raw(
			"access-control-allow-headers",
			vec![b"Content-Type".to_vec()],
		);
		let message = format!("{}", e.kind());
		let mut r = Response::with((status::InternalServerError, message));
		r.headers = headers;
		Ok(r)
	}

	fn create_ok_response(&self, json: &str) -> IronResult<Response> {
		let mut headers = Headers::new();
		headers.set_raw("access-control-allow-origin", vec![b"*".to_vec()]);
		let mut r = Response::with((status::Ok, json));
		r.headers = headers;
		Ok(r)
	}
}

impl<T: ?Sized, C, K> Handler for OwnerAPIPostHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let mut api = APIOwner::new(self.wallet.clone());
		let resp = match self.handle_request(req, &mut api) {
			Ok(r) => self.create_ok_response(&r),
			Err(e) => {
				error!(LOGGER, "Request Error: {:?}", e);
				self.create_error_response(e)
			}
		};
		resp
	}
}

/// Options handler
pub struct OwnerAPIOptionsHandler {}

impl Handler for OwnerAPIOptionsHandler where {
	fn handle(&self, _req: &mut Request) -> IronResult<Response> {
		let mut resp_json = Ok(Response::with((status::Ok, "{}")));
		let mut headers = Headers::new();
		headers.set_raw("access-control-allow-origin", vec![b"*".to_vec()]);
		headers.set_raw(
			"access-control-allow-headers",
			vec![b"Content-Type".to_vec()],
		);
		resp_json.as_mut().unwrap().headers = headers;
		resp_json
	}
}
/// API Handler/Wrapper for foreign functions

pub struct ForeignAPIHandler<T: ?Sized, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<Box<T>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> ForeignAPIHandler<T, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// create a new api handler
	pub fn new(wallet: Arc<Mutex<Box<T>>>) -> ForeignAPIHandler<T, C, K> {
		ForeignAPIHandler {
			wallet,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	fn build_coinbase(
		&self,
		req: &mut Request,
		api: &mut APIForeign<T, C, K>,
	) -> Result<CbData, Error> {
		let struct_body = req.get::<bodyparser::Struct<BlockFees>>();
		match struct_body {
			Ok(Some(block_fees)) => api.build_coinbase(&block_fees),
			Ok(None) => {
				error!(LOGGER, "Missing request body: build_coinbase");
				Err(ErrorKind::GenericError(
					"Invalid request body: build_coinbase".to_owned(),
				))?
			}
			Err(e) => {
				error!(LOGGER, "Invalid request body: build_coinbase: {:?}", e);
				Err(ErrorKind::GenericError(
					"Invalid request body: build_coinbase".to_owned(),
				))?
			}
		}
	}

	fn receive_tx(&self, req: &mut Request, api: &mut APIForeign<T, C, K>) -> Result<Slate, Error> {
		let struct_body = req.get::<bodyparser::Struct<Slate>>();
		if let Ok(Some(mut slate)) = struct_body {
			api.receive_tx(&mut slate)?;
			Ok(slate.clone())
		} else {
			Err(ErrorKind::GenericError(
				"Invalid request body: receive_tx".to_owned(),
			))?
		}
	}

	fn handle_request(
		&self,
		req: &mut Request,
		api: &mut APIForeign<T, C, K>,
	) -> IronResult<Response> {
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
impl<T: ?Sized, C, K> Handler for ForeignAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + Send + Sync + 'static,
	K: Keychain + 'static,
{
	fn handle(&self, req: &mut Request) -> IronResult<Response> {
		let mut api = APIForeign::new(self.wallet.clone());
		let resp_json = self.handle_request(req, &mut *api);
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
fn json_response_pretty<T>(s: &T) -> Result<String, Error>
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => Ok(json),
		Err(_) => Err(ErrorKind::Format)?,
	}
}
