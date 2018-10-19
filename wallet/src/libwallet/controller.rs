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
use api::{ApiServer, BasicAuthMiddleware, Handler, ResponseFuture, Router, TLSConfig};
use core::core::Transaction;
use failure::ResultExt;
use futures::future::{err, ok};
use futures::{Future, Stream};
use hyper::{Body, Request, Response, StatusCode};
use keychain::Keychain;
use libtx::slate::Slate;
use libwallet::api::{APIForeign, APIOwner};
use libwallet::types::{
	CbData, OutputData, SendTXArgs, TxLogEntry, WalletBackend, WalletClient, WalletInfo,
};
use libwallet::{Error, ErrorKind};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;
use url::form_urlencoded;
use util::secp::pedersen;
use util::Mutex;
use util::{to_base64, LOGGER};

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
pub fn owner_listener<T: ?Sized, C, K>(
	wallet: Box<T>,
	addr: &str,
	api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
) -> Result<(), Error>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	OwnerAPIHandler<T, C, K>: Handler,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	let wallet_arc = Arc::new(Mutex::new(wallet));
	let api_handler = OwnerAPIHandler::new(wallet_arc);

	let mut router = Router::new();
	if api_secret.is_some() {
		let api_basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret.unwrap()));
		let basic_realm = "Basic realm=GrinOwnerAPI".to_string();
		let basic_auth_middleware = Arc::new(BasicAuthMiddleware::new(api_basic_auth, basic_realm));
		router.add_middleware(basic_auth_middleware);
	}
	router
		.add_route("/v1/wallet/owner/**", Arc::new(api_handler))
		.map_err(|_| ErrorKind::GenericError("Router failed to add route".to_string()))?;

	let mut apis = ApiServer::new();
	info!(LOGGER, "Starting HTTP Owner API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread =
		apis.start(socket_addr, router, tls_config)
			.context(ErrorKind::GenericError(
				"API thread failed to start".to_string(),
			))?;
	api_thread
		.join()
		.map_err(|e| ErrorKind::GenericError(format!("API thread panicked :{:?}", e)).into())
}

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn foreign_listener<T: ?Sized, C, K>(
	wallet: Box<T>,
	addr: &str,
	tls_config: Option<TLSConfig>,
) -> Result<(), Error>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	let api_handler = ForeignAPIHandler::new(Arc::new(Mutex::new(wallet)));

	let mut router = Router::new();
	router
		.add_route("/v1/wallet/foreign/**", Arc::new(api_handler))
		.map_err(|_| ErrorKind::GenericError("Router failed to add route".to_string()))?;

	let mut apis = ApiServer::new();
	info!(LOGGER, "Starting HTTP Foreign API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread =
		apis.start(socket_addr, router, tls_config)
			.context(ErrorKind::GenericError(
				"API thread failed to start".to_string(),
			))?;

	api_thread
		.join()
		.map_err(|e| ErrorKind::GenericError(format!("API thread panicked :{:?}", e)).into())
}

type WalletResponseFuture = Box<Future<Item = Response<Body>, Error = Error> + Send>;

/// API Handler/Wrapper for owner functions
pub struct OwnerAPIHandler<T: ?Sized, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<Box<T>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> OwnerAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
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
		api: APIOwner<T, C, K>,
	) -> Result<(bool, Vec<(OutputData, pedersen::Commitment)>), Error> {
		let mut update_from_node = false;
		let mut id = None;
		let mut show_spent = false;
		let params = parse_params(req);

		if let Some(_) = params.get("refresh") {
			update_from_node = true;
		}
		if let Some(_) = params.get("show_spent") {
			show_spent = true;
		}
		if let Some(ids) = params.get("tx_id") {
			for i in ids {
				id = Some(i.parse().unwrap());
			}
		}
		api.retrieve_outputs(show_spent, update_from_node, id)
	}

	fn retrieve_txs(
		&self,
		req: &Request<Body>,
		api: APIOwner<T, C, K>,
	) -> Result<(bool, Vec<TxLogEntry>), Error> {
		let mut id = None;
		let mut update_from_node = false;

		let params = parse_params(req);

		if let Some(_) = params.get("refresh") {
			update_from_node = true;
		}
		if let Some(ids) = params.get("id") {
			for i in ids {
				id = Some(i.parse().unwrap());
			}
		}
		api.retrieve_txs(update_from_node, id)
	}

	fn dump_stored_tx(
		&self,
		req: &Request<Body>,
		api: APIOwner<T, C, K>,
	) -> Result<Transaction, Error> {
		let params = parse_params(req);
		if let Some(id_string) = params.get("id") {
			match id_string[0].parse() {
				Ok(id) => match api.dump_stored_tx(id, false, "") {
					Ok(tx) => Ok(tx),
					Err(e) => {
						error!(LOGGER, "dump_stored_tx: failed with error: {}", e);
						Err(e)
					}
				},
				Err(e) => {
					error!(LOGGER, "dump_stored_tx: could not parse id: {}", e);
					Err(ErrorKind::TransactionDumpError(
						"dump_stored_tx: cannot dump transaction. Could not parse id in request.",
					).into())
				}
			}
		} else {
			Err(ErrorKind::TransactionDumpError(
				"dump_stored_tx: Cannot dump transaction. Missing id param in request.",
			).into())
		}
	}

	fn retrieve_summary_info(
		&self,
		req: &Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Result<(bool, WalletInfo), Error> {
		let update_from_node = param_exists(req, "refresh");
		api.retrieve_summary_info(update_from_node)
	}

	fn node_height(
		&self,
		_req: &Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Result<(u64, bool), Error> {
		api.node_height()
	}

	fn handle_get_request(&self, req: &Request<Body>) -> Result<Response<Body>, Error> {
		let api = APIOwner::new(self.wallet.clone());

		Ok(match req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"retrieve_outputs" => json_response(&self.retrieve_outputs(req, api)?),
			"retrieve_summary_info" => json_response(&self.retrieve_summary_info(req, api)?),
			"node_height" => json_response(&self.node_height(req, api)?),
			"retrieve_txs" => json_response(&self.retrieve_txs(req, api)?),
			"dump_stored_tx" => json_response(&self.dump_stored_tx(req, api)?),
			_ => response(StatusCode::BAD_REQUEST, ""),
		})
	}

	fn issue_send_tx(
		&self,
		req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<Future<Item = Slate, Error = Error> + Send> {
		Box::new(parse_body(req).and_then(move |args: SendTXArgs| {
			if args.method == "http" {
				api.issue_send_tx(
					args.amount,
					args.minimum_confirmations,
					&args.dest,
					args.max_outputs,
					args.num_change_outputs,
					args.selection_strategy_is_use_all,
				)
			} else if args.method == "file" {
				api.send_tx(
					false,
					args.amount,
					args.minimum_confirmations,
					&args.dest,
					args.max_outputs,
					args.num_change_outputs,
					args.selection_strategy_is_use_all,
				)
			} else {
				error!(LOGGER, "unsupported payment method: {}", args.method);
				return Err(ErrorKind::ClientCallback("unsupported payment method"))?;
			}
		}))
	}

	fn finalize_tx(
		&self,
		req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<Future<Item = Slate, Error = Error> + Send> {
		Box::new(
			parse_body(req).and_then(move |mut slate| match api.finalize_tx(&mut slate) {
				Ok(_) => ok(slate.clone()),
				Err(e) => {
					error!(LOGGER, "finalize_tx: failed with error: {}", e);
					err(e)
				}
			}),
		)
	}

	fn cancel_tx(
		&self,
		req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<Future<Item = (), Error = Error> + Send> {
		let params = parse_params(&req);
		if let Some(id_string) = params.get("id") {
			Box::new(match id_string[0].parse() {
				Ok(id) => match api.cancel_tx(id) {
					Ok(_) => ok(()),
					Err(e) => {
						error!(LOGGER, "cancel_tx: failed with error: {}", e);
						err(e)
					}
				},
				Err(e) => {
					error!(LOGGER, "cancel_tx: could not parse id: {}", e);
					err(ErrorKind::TransactionCancellationError(
						"cancel_tx: cannot cancel transaction. Could not parse id in request.",
					).into())
				}
			})
		} else {
			Box::new(err(ErrorKind::TransactionCancellationError(
				"cancel_tx: Cannot cancel transaction. Missing id param in request.",
			).into()))
		}
	}

	fn issue_burn_tx(
		&self,
		_req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<Future<Item = (), Error = Error> + Send> {
		// TODO: Args
		Box::new(match api.issue_burn_tx(60, 10, 1000) {
			Ok(_) => ok(()),
			Err(e) => err(e),
		})
	}

	fn handle_post_request(&self, req: Request<Body>) -> WalletResponseFuture {
		let api = APIOwner::new(self.wallet.clone());
		match req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"issue_send_tx" => Box::new(
				self.issue_send_tx(req, api)
					.and_then(|slate| ok(json_response_pretty(&slate))),
			),
			"finalize_tx" => Box::new(
				self.finalize_tx(req, api)
					.and_then(|slate| ok(json_response_pretty(&slate))),
			),
			"cancel_tx" => Box::new(
				self.cancel_tx(req, api)
					.and_then(|_| ok(response(StatusCode::OK, ""))),
			),
			"issue_burn_tx" => Box::new(
				self.issue_burn_tx(req, api)
					.and_then(|_| ok(response(StatusCode::OK, ""))),
			),
			_ => Box::new(err(ErrorKind::GenericError(
				"Unknown error handling post request".to_owned(),
			).into())),
		}
	}
}

impl<T: ?Sized, C, K> Handler for OwnerAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		match self.handle_get_request(&req) {
			Ok(r) => Box::new(ok(r)),
			Err(e) => {
				error!(LOGGER, "Request Error: {:?}", e);
				Box::new(ok(create_error_response(e)))
			}
		}
	}

	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(
			self.handle_post_request(req)
				.and_then(|r| ok(r))
				.or_else(|e| {
					error!(LOGGER, "Request Error: {:?}", e);
					ok(create_error_response(e))
				}),
		)
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::new(ok(create_ok_response("{}")))
	}
}

/// API Handler/Wrapper for foreign functions

pub struct ForeignAPIHandler<T: ?Sized, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<Box<T>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> ForeignAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + 'static,
	K: Keychain + 'static,
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
		mut api: APIForeign<T, C, K>,
	) -> Box<Future<Item = CbData, Error = Error> + Send> {
		Box::new(parse_body(req).and_then(move |block_fees| api.build_coinbase(&block_fees)))
	}

	fn receive_tx(
		&self,
		req: Request<Body>,
		mut api: APIForeign<T, C, K>,
	) -> Box<Future<Item = Slate, Error = Error> + Send> {
		Box::new(
			parse_body(req).and_then(move |mut slate| match api.receive_tx(&mut slate) {
				Ok(_) => ok(slate.clone()),
				Err(e) => {
					error!(LOGGER, "receive_tx: failed with error: {}", e);
					err(e)
				}
			}),
		)
	}

	fn handle_request(&self, req: Request<Body>) -> WalletResponseFuture {
		let api = *APIForeign::new(self.wallet.clone());
		match req
			.uri()
			.path()
			.trim_right_matches("/")
			.rsplit("/")
			.next()
			.unwrap()
		{
			"build_coinbase" => Box::new(
				self.build_coinbase(req, api)
					.and_then(|res| ok(json_response(&res))),
			),
			"receive_tx" => Box::new(
				self.receive_tx(req, api)
					.and_then(|res| ok(json_response(&res))),
			),
			_ => Box::new(ok(response(StatusCode::BAD_REQUEST, "unknown action"))),
		}
	}
}
impl<T: ?Sized, C, K> Handler for ForeignAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: WalletClient + Send + Sync + 'static,
	K: Keychain + 'static,
{
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(self.handle_request(req).and_then(|r| ok(r)).or_else(|e| {
			error!(LOGGER, "Request Error: {:?}", e);
			ok(create_error_response(e))
		}))
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
		.body(format!("{}", e).into())
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
}

fn parse_params(req: &Request<Body>) -> HashMap<String, Vec<String>> {
	match req.uri().query() {
		Some(query_string) => form_urlencoded::parse(query_string.as_bytes())
			.into_owned()
			.fold(HashMap::new(), |mut hm, (k, v)| {
				hm.entry(k).or_insert(vec![]).push(v);
				hm
			}),
		None => HashMap::new(),
	}
}

fn param_exists(req: &Request<Body>, param: &str) -> bool {
	parse_params(req).get(param).is_some()
}

fn parse_body<T>(req: Request<Body>) -> Box<Future<Item = T, Error = Error> + Send>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	Box::new(
		req.into_body()
			.concat2()
			.map_err(|_| ErrorKind::GenericError("Failed to read request".to_owned()).into())
			.and_then(|body| match serde_json::from_reader(&body.to_vec()[..]) {
				Ok(obj) => ok(obj),
				Err(e) => {
					err(ErrorKind::GenericError(format!("Invalid request body: {}", e)).into())
				}
			}),
	)
}
