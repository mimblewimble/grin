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
use api::{ApiServer, Handler, Router};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use futures::Stream;
use hyper::header::HeaderMap;
use hyper::{Body, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;
use tokio_core::reactor::Core;

use keychain::Keychain;
use libtx::slate::Slate;
use libwallet::api::{APIForeign, APIOwner};
use libwallet::types::{
	BlockFees, CbData, OutputData, SendTXArgs, WalletBackend, WalletClient, WalletInfo,
};
use libwallet::{Error, ErrorKind};
use url::form_urlencoded;

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

static mut OWNER_ROUTER: Option<Router> = None;

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn owner_listener<T: ?Sized, C, K>(wallet: Box<T>, addr: &str) -> Result<(), Error>
where
	T: WalletBackend<C, K> + 'static,
	OwnerAPIHandler<T, C, K>: Handler,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	let wallet_arc = Arc::new(Mutex::new(wallet));
	let api_handler = OwnerAPIHandler::new(wallet_arc);

	let mut router = Router::new();
	router
		.add_route("/v1/wallet/owner/**", Box::new(api_handler))
		.map_err(|_e| ErrorKind::GenericError("Router failed to add route".to_string()))?;
	unsafe {
		OWNER_ROUTER = Some(router);
	}

	let mut apis = ApiServer::new();
	info!(LOGGER, "Starting HTTP Owner API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	apis.start(socket_addr, &handle_owner).unwrap_or_else(|e| {
		error!(LOGGER, "Failed to start API HTTP server: {}.", e);
	});
	Ok(())
}

pub fn handle_owner(req: Request<Body>) -> Response<Body> {
	unsafe {
		match OWNER_ROUTER {
			Some(ref h) => h.handle(req),
			None => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
		}
	}
}

static mut FOREIGN_ROUTER: Option<Router> = None;

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn foreign_listener<T: ?Sized, C, K>(wallet: Box<T>, addr: &str) -> Result<(), Error>
where
	T: WalletBackend<C, K> + 'static,
	ForeignAPIHandler<T, C, K>: Handler,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	let api_handler = ForeignAPIHandler::new(Arc::new(Mutex::new(wallet)));

	let mut router = Router::new();
	router
		.add_route("/v1/wallet/foreign/**", Box::new(api_handler))
		.map_err(|_e| ErrorKind::GenericError("Router failed to add route".to_string()))?;

	unsafe {
		FOREIGN_ROUTER = Some(router);
	}

	let mut apis = ApiServer::new();
	info!(LOGGER, "Starting HTTP Foreign API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	apis.start(socket_addr, &handle_foreign)
		.unwrap_or_else(|e| {
			error!(LOGGER, "Failed to start API HTTP server: {}.", e);
		});

	Ok(())
}

pub fn handle_foreign(req: Request<Body>) -> Response<Body> {
	unsafe {
		match FOREIGN_ROUTER {
			Some(ref h) => h.handle(req),
			None => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
		}
	}
}
/// API Handler/Wrapper for owner functions

pub struct OwnerAPIHandler<T: ?Sized, C, K>
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

impl<T: ?Sized, C, K> OwnerAPIHandler<T, C, K>
where
	T: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Create a new owner API handler for GET methods
	pub fn new(wallet: Arc<Mutex<Box<T>>>) -> OwnerAPIHandler<T, C, K> {
		OwnerAPIHandler {
			wallet,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	fn retrieve_outputs(
		&self,
		req: &Request<Body>,
		api: &mut APIOwner<T, C, K>,
	) -> Result<(bool, Vec<OutputData>), Error> {
		let update_from_node = param_exists(req, "refresh");
		api.retrieve_outputs(false, update_from_node)
	}

	fn retrieve_summary_info(
		&self,
		req: &Request<Body>,
		api: &mut APIOwner<T, C, K>,
	) -> Result<(bool, WalletInfo), Error> {
		let update_from_node = param_exists(req, "refresh");
		api.retrieve_summary_info(update_from_node)
	}

	fn node_height(
		&self,
		_req: &Request<Body>,
		api: &mut APIOwner<T, C, K>,
	) -> Result<(u64, bool), Error> {
		api.node_height()
	}

	fn handle_get_request(
		&self,
		req: &Request<Body>,
		api: &mut APIOwner<T, C, K>,
	) -> Result<Response<Body>, Error> {
		Ok(match req.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"retrieve_outputs" => json_response(&self.retrieve_outputs(req, api)?),
			"retrieve_summary_info" => json_response(&self.retrieve_summary_info(req, api)?),
			"node_height" => json_response(&self.node_height(req, api)?),
			_ => response(StatusCode::BAD_REQUEST, ""),
		})
	}

	fn issue_send_tx(
		&self,
		req: Request<Body>,
		api: &mut APIOwner<T, C, K>,
	) -> Result<Slate, Error> {
		let args: SendTXArgs = parse_body(req)?;
		api.issue_send_tx(
			args.amount,
			args.minimum_confirmations,
			&args.dest,
			args.max_outputs,
			args.selection_strategy_is_use_all,
		)
	}

	fn issue_burn_tx(&self, _req: Request<Body>, api: &mut APIOwner<T, C, K>) -> Result<(), Error> {
		// TODO: Args
		api.issue_burn_tx(60, 10, 1000)
	}

	fn handle_post_request(
		&self,
		req: Request<Body>,
		api: &mut APIOwner<T, C, K>,
	) -> Result<Response<Body>, Error> {
		Ok(match req.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"issue_send_tx" => json_response_pretty(&self.issue_send_tx(req, api)?),
			"issue_burn_tx" => json_response_pretty(&self.issue_burn_tx(req, api)?),
			_ => Err(ErrorKind::GenericError(
				"Unknown error handling post request".to_owned(),
			))?,
		})
	}
}

impl<T: ?Sized, C, K> Handler for OwnerAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	fn get(&self, req: Request<Body>) -> Response<Body> {
		let mut api = APIOwner::new(self.wallet.clone());
		match self.handle_get_request(&req, &mut api) {
			Ok(r) => r,
			Err(e) => {
				error!(LOGGER, "Request Error: {:?}", e);
				create_error_response(e)
			}
		}
	}

	fn post(&self, req: Request<Body>) -> Response<Body> {
		let mut api = APIOwner::new(self.wallet.clone());
		match self.handle_post_request(req, &mut api) {
			Ok(r) => r,
			Err(e) => {
				error!(LOGGER, "Request Error: {:?}", e);
				create_error_response(e)
			}
		}
	}

	fn options(&self, _req: Request<Body>) -> Response<Body> {
		create_ok_response("{}")
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
		req: Request<Body>,
		api: &mut APIForeign<T, C, K>,
	) -> Result<CbData, Error> {
		let block_fees = parse_body(req)?;
		api.build_coinbase(&block_fees)
	}

	fn receive_tx(
		&self,
		req: Request<Body>,
		api: &mut APIForeign<T, C, K>,
	) -> Result<Slate, Error> {
		let mut slate = parse_body(req)?;
		api.receive_tx(&mut slate)?;
		Ok(slate.clone())
	}

	fn handle_request(
		&self,
		req: Request<Body>,
		api: &mut APIForeign<T, C, K>,
	) -> Result<Response<Body>, Error> {
		Ok(match req.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"build_coinbase" => json_response(&self.build_coinbase(req, api)?),
			"receive_tx" => json_response(&self.receive_tx(req, api)?),
			_ => response(StatusCode::BAD_REQUEST, "unknown action"),
		})
	}
}
impl<T: ?Sized, C, K> Handler for ForeignAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + Send + Sync + 'static,
	K: Keychain + 'static,
{
	fn post(&self, req: Request<Body>) -> Response<Body> {
		let mut api = APIForeign::new(self.wallet.clone());
		match self.handle_request(req, &mut *api) {
			Ok(r) => r,
			Err(e) => create_error_response(e),
		}
	}
}

// Utility to serialize a struct into JSON and produce a sensible Response
// out of it.
fn json_response<T>(s: &T) -> Response<Body>
where
	T: Serialize,
{
	match serde_json::to_string(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

// pretty-printed version of above
fn json_response_pretty<T>(s: &T) -> Response<Body>
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

fn create_error_response(e: Error) -> Response<Body> {
	Response::builder()
		.status(StatusCode::INTERNAL_SERVER_ERROR)
		.header("access-control-allow-origin", "*")
		.header("access-control-allow-headers", "Content-Type")
		.body(format!("{}", e.kind()).into())
		.unwrap()
}

fn create_ok_response(json: &str) -> Response<Body> {
	Response::builder()
		.status(StatusCode::OK)
		.header("access-control-allow-origin", "*")
		.body(json.to_string().into())
		.unwrap()
}

fn response<T: Into<Body>>(status: StatusCode, text: T) -> Response<Body> {
	Response::builder()
		.status(status)
		.header("access-control-allow-origin", "*")
		.body(text.into())
		.unwrap()
	//let mut resp = Response::new(text.into());
	//*resp.status_mut() = status;
	//resp
}

fn param_exists(req: &Request<Body>, param: &str) -> bool {
	if let Some(query_string) = req.uri().query() {
		let params = form_urlencoded::parse(query_string.as_bytes())
			.into_owned()
			.fold(HashMap::new(), |mut hm, (k, v)| {
				hm.entry(k).or_insert(vec![]).push(v);
				hm
			});
		return params.get(param).is_some();
	}
	false
}

fn parse_body<T>(req: Request<Body>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	let mut event_loop = Core::new().unwrap();
	let task = req.into_body().concat2();
	let body = event_loop
		.run(task)
		.map_err(|_e| ErrorKind::GenericError("Failed to read request body".to_owned()))?;
	let obj: T = serde_json::from_reader(&body.to_vec()[..])
		.map_err(|_e| ErrorKind::GenericError("Invalid request body".to_owned()))?;
	Ok(obj)
}
