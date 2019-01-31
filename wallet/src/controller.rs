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
use crate::adapters::{FileWalletCommAdapter, HTTPWalletCommAdapter, KeybaseWalletCommAdapter};
use crate::api::{ApiServer, BasicAuthMiddleware, Handler, ResponseFuture, Router, TLSConfig};
use crate::core::core;
use crate::core::core::Transaction;
use crate::core::libtx::slate::Slate;
use crate::keychain::Keychain;
use crate::libwallet::api::{APIForeign, APIOwner};
use crate::libwallet::types::{
	CbData, NodeClient, OutputData, SendTXArgs, TxLogEntry, WalletBackend, WalletInfo,
};
use crate::libwallet::{Error, ErrorKind};
use crate::util::secp::pedersen;
use crate::util::to_base64;
use crate::util::Mutex;
use failure::ResultExt;
use futures::future::{err, ok};
use futures::{Future, Stream};
use hyper::{Body, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;
use url::form_urlencoded;

/// Instantiate wallet Owner API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn owner_single_use<F, T: ?Sized, C, K>(wallet: Arc<Mutex<T>>, f: F) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	F: FnOnce(&mut APIOwner<T, C, K>) -> Result<(), Error>,
	C: NodeClient,
	K: Keychain,
{
	f(&mut APIOwner::new(wallet.clone()))?;
	Ok(())
}

/// Instantiate wallet Foreign API for a single-use (command line) call
/// Return a function containing a loaded API context to call
pub fn foreign_single_use<F, T: ?Sized, C, K>(wallet: Arc<Mutex<T>>, f: F) -> Result<(), Error>
where
	T: WalletBackend<C, K>,
	F: FnOnce(&mut APIForeign<T, C, K>) -> Result<(), Error>,
	C: NodeClient,
	K: Keychain,
{
	f(&mut APIForeign::new(wallet.clone()))?;
	Ok(())
}

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn owner_listener<T: ?Sized, C, K>(
	wallet: Arc<Mutex<T>>,
	addr: &str,
	api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
	owner_api_include_foreign: Option<bool>,
) -> Result<(), Error>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	OwnerAPIHandler<T, C, K>: Handler,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	let api_handler = OwnerAPIHandler::new(wallet.clone());

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

	// If so configured, add the foreign API to the same port
	if owner_api_include_foreign.unwrap_or(false) {
		info!("Starting HTTP Foreign API on Owner server at {}.", addr);
		let foreign_api_handler = ForeignAPIHandler::new(wallet.clone());
		router
			.add_route("/v1/wallet/foreign/**", Arc::new(foreign_api_handler))
			.map_err(|_| ErrorKind::GenericError("Router failed to add route".to_string()))?;
	}

	let mut apis = ApiServer::new();
	info!("Starting HTTP Owner API server at {}.", addr);
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
	wallet: Arc<Mutex<T>>,
	addr: &str,
	tls_config: Option<TLSConfig>,
) -> Result<(), Error>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	let api_handler = ForeignAPIHandler::new(wallet);

	let mut router = Router::new();
	router
		.add_route("/v1/wallet/foreign/**", Arc::new(api_handler))
		.map_err(|_| ErrorKind::GenericError("Router failed to add route".to_string()))?;

	let mut apis = ApiServer::new();
	warn!("Starting HTTP Foreign listener API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread =
		apis.start(socket_addr, router, tls_config)
			.context(ErrorKind::GenericError(
				"API thread failed to start".to_string(),
			))?;
	warn!("HTTP Foreign listener started.");

	api_thread
		.join()
		.map_err(|e| ErrorKind::GenericError(format!("API thread panicked :{:?}", e)).into())
}

type WalletResponseFuture = Box<dyn Future<Item = Response<Body>, Error = Error> + Send>;

/// API Handler/Wrapper for owner functions
pub struct OwnerAPIHandler<T: ?Sized, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<T>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> OwnerAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	/// Create a new owner API handler for GET methods
	pub fn new(wallet: Arc<Mutex<T>>) -> OwnerAPIHandler<T, C, K> {
		OwnerAPIHandler {
			wallet,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	pub fn retrieve_outputs(
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
			if let Some(x) = ids.first() {
				id = Some(x.parse().unwrap());
			}
		}
		api.retrieve_outputs(show_spent, update_from_node, id)
	}

	pub fn retrieve_txs(
		&self,
		req: &Request<Body>,
		api: APIOwner<T, C, K>,
	) -> Result<(bool, Vec<TxLogEntry>), Error> {
		let mut tx_id = None;
		let mut tx_slate_id = None;
		let mut update_from_node = false;

		let params = parse_params(req);

		if let Some(_) = params.get("refresh") {
			update_from_node = true;
		}
		if let Some(ids) = params.get("id") {
			if let Some(x) = ids.first() {
				tx_id = Some(x.parse().unwrap());
			}
		}
		if let Some(tx_slate_ids) = params.get("tx_id") {
			if let Some(x) = tx_slate_ids.first() {
				tx_slate_id = Some(x.parse().unwrap());
			}
		}
		api.retrieve_txs(update_from_node, tx_id, tx_slate_id)
	}

	pub fn retrieve_stored_tx(
		&self,
		req: &Request<Body>,
		api: APIOwner<T, C, K>,
	) -> Result<(bool, Option<Transaction>), Error> {
		let params = parse_params(req);
		if let Some(id_string) = params.get("id") {
			match id_string[0].parse() {
				Ok(id) => match api.retrieve_txs(true, Some(id), None) {
					Ok((_, txs)) => {
						let stored_tx = api.get_stored_tx(&txs[0])?;
						Ok((txs[0].confirmed, stored_tx))
					}
					Err(e) => {
						error!("retrieve_stored_tx: failed with error: {}", e);
						Err(e)
					}
				},
				Err(e) => {
					error!("retrieve_stored_tx: could not parse id: {}", e);
					Err(ErrorKind::TransactionDumpError(
						"retrieve_stored_tx: cannot dump transaction. Could not parse id in request.",
					).into())
				}
			}
		} else {
			Err(ErrorKind::TransactionDumpError(
				"retrieve_stored_tx: Cannot retrieve transaction. Missing id param in request.",
			)
			.into())
		}
	}

	pub fn retrieve_summary_info(
		&self,
		req: &Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Result<(bool, WalletInfo), Error> {
		let mut minimum_confirmations = 1; // TODO - default needed here
		let params = parse_params(req);
		let update_from_node = params.get("refresh").is_some();

		if let Some(confs) = params.get("minimum_confirmations") {
			if let Some(x) = confs.first() {
				minimum_confirmations = x.parse().unwrap();
			}
		}

		api.retrieve_summary_info(update_from_node, minimum_confirmations)
	}

	pub fn node_height(
		&self,
		_req: &Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Result<(u64, bool), Error> {
		api.node_height()
	}

	fn handle_get_request(&self, req: &Request<Body>) -> Result<Response<Body>, Error> {
		let api = APIOwner::new(self.wallet.clone());

		Ok(
			match req
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
				"retrieve_stored_tx" => json_response(&self.retrieve_stored_tx(req, api)?),
				_ => response(StatusCode::BAD_REQUEST, ""),
			},
		)
	}

	pub fn issue_send_tx(
		&self,
		req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<dyn Future<Item = Slate, Error = Error> + Send> {
		Box::new(parse_body(req).and_then(move |args: SendTXArgs| {
			let result = api.initiate_tx(
				None,
				args.amount,
				args.minimum_confirmations,
				args.max_outputs,
				args.num_change_outputs,
				args.selection_strategy_is_use_all,
				args.message,
			);
			let (mut slate, lock_fn) = match result {
				Ok(s) => {
					info!(
						"Tx created: {} grin to {} (strategy '{}')",
						core::amount_to_hr_string(args.amount, false),
						&args.dest,
						args.selection_strategy_is_use_all,
					);
					s
				}
				Err(e) => {
					error!("Tx not created: {}", e);
					match e.kind() {
						// user errors, don't backtrace
						ErrorKind::NotEnoughFunds { .. } => {}
						ErrorKind::FeeDispute { .. } => {}
						ErrorKind::FeeExceedsAmount { .. } => {}
						_ => {
							// otherwise give full dump
							error!("Backtrace: {}", e.backtrace().unwrap());
						}
					};
					return Err(e);
				}
			};
			match args.method.as_ref() {
				"http" => slate = HTTPWalletCommAdapter::new().send_tx_sync(&args.dest, &slate)?,
				"file" => {
					FileWalletCommAdapter::new().send_tx_async(&args.dest, &slate)?;
				}
				"keybase" => {
					//TODO: in case of keybase, the response might take 60s and leave the service hanging
					slate = KeybaseWalletCommAdapter::new().send_tx_sync(&args.dest, &slate)?;
				}
				_ => {
					error!("unsupported payment method: {}", args.method);
					return Err(ErrorKind::ClientCallback(
						"unsupported payment method".to_owned(),
					))?;
				}
			}
			api.tx_lock_outputs(&slate, lock_fn)?;
			if args.method != "file" {
				api.finalize_tx(&mut slate)?;
			}
			Ok(slate)
		}))
	}

	pub fn finalize_tx(
		&self,
		req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<dyn Future<Item = Slate, Error = Error> + Send> {
		Box::new(
			parse_body(req).and_then(move |mut slate| match api.finalize_tx(&mut slate) {
				Ok(_) => ok(slate.clone()),
				Err(e) => {
					error!("finalize_tx: failed with error: {}", e);
					err(e)
				}
			}),
		)
	}

	pub fn cancel_tx(
		&self,
		req: Request<Body>,
		mut api: APIOwner<T, C, K>,
	) -> Box<dyn Future<Item = (), Error = Error> + Send> {
		let params = parse_params(&req);
		if let Some(id_string) = params.get("id") {
			Box::new(match id_string[0].parse() {
				Ok(id) => match api.cancel_tx(Some(id), None) {
					Ok(_) => ok(()),
					Err(e) => {
						error!("cancel_tx: failed with error: {}", e);
						err(e)
					}
				},
				Err(e) => {
					error!("cancel_tx: could not parse id: {}", e);
					err(ErrorKind::TransactionCancellationError(
						"cancel_tx: cannot cancel transaction. Could not parse id in request.",
					)
					.into())
				}
			})
		} else if let Some(tx_id_string) = params.get("tx_id") {
			Box::new(match tx_id_string[0].parse() {
				Ok(tx_id) => match api.cancel_tx(None, Some(tx_id)) {
					Ok(_) => ok(()),
					Err(e) => {
						error!("cancel_tx: failed with error: {}", e);
						err(e)
					}
				},
				Err(e) => {
					error!("cancel_tx: could not parse tx_id: {}", e);
					err(ErrorKind::TransactionCancellationError(
						"cancel_tx: cannot cancel transaction. Could not parse tx_id in request.",
					)
					.into())
				}
			})
		} else {
			Box::new(err(ErrorKind::TransactionCancellationError(
				"cancel_tx: Cannot cancel transaction. Missing id or tx_id param in request.",
			)
			.into()))
		}
	}

	pub fn post_tx(
		&self,
		req: Request<Body>,
		api: APIOwner<T, C, K>,
	) -> Box<dyn Future<Item = (), Error = Error> + Send> {
		let params = match req.uri().query() {
			Some(query_string) => form_urlencoded::parse(query_string.as_bytes())
				.into_owned()
				.fold(HashMap::new(), |mut hm, (k, v)| {
					hm.entry(k).or_insert(vec![]).push(v);
					hm
				}),
			None => HashMap::new(),
		};
		let fluff = params.get("fluff").is_some();
		Box::new(parse_body(req).and_then(
			move |slate: Slate| match api.post_tx(&slate.tx, fluff) {
				Ok(_) => ok(()),
				Err(e) => {
					error!("post_tx: failed with error: {}", e);
					err(e)
				}
			},
		))
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
			"post_tx" => Box::new(
				self.post_tx(req, api)
					.and_then(|_| ok(response(StatusCode::OK, ""))),
			),
			_ => Box::new(err(ErrorKind::GenericError(
				"Unknown error handling post request".to_owned(),
			)
			.into())),
		}
	}
}

impl<T: ?Sized, C, K> Handler for OwnerAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	fn get(&self, req: Request<Body>) -> ResponseFuture {
		match self.handle_get_request(&req) {
			Ok(r) => Box::new(ok(r)),
			Err(e) => {
				error!("Request Error: {:?}", e);
				Box::new(ok(create_error_response(e)))
			}
		}
	}

	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(
			self.handle_post_request(req)
				.and_then(|r| ok(r))
				.or_else(|e| {
					error!("Request Error: {:?}", e);
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
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	/// Wallet instance
	pub wallet: Arc<Mutex<T>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<T: ?Sized, C, K> ForeignAPIHandler<T, C, K>
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient + 'static,
	K: Keychain + 'static,
{
	/// create a new api handler
	pub fn new(wallet: Arc<Mutex<T>>) -> ForeignAPIHandler<T, C, K> {
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
	) -> Box<dyn Future<Item = CbData, Error = Error> + Send> {
		Box::new(parse_body(req).and_then(move |block_fees| api.build_coinbase(&block_fees)))
	}

	fn receive_tx(
		&self,
		req: Request<Body>,
		mut api: APIForeign<T, C, K>,
	) -> Box<dyn Future<Item = Slate, Error = Error> + Send> {
		Box::new(parse_body(req).and_then(
			//TODO: No way to insert a message from the params
			move |mut slate| {
				if let Err(e) = api.verify_slate_messages(&slate) {
					error!("Error validating participant messages: {}", e);
					err(e)
				} else {
					match api.receive_tx(&mut slate, None, None) {
						Ok(_) => ok(slate.clone()),
						Err(e) => {
							error!("receive_tx: failed with error: {}", e);
							err(e)
						}
					}
				}
			},
		))
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
	C: NodeClient + Send + Sync + 'static,
	K: Keychain + 'static,
{
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(self.handle_request(req).and_then(|r| ok(r)).or_else(|e| {
			error!("Request Error: {:?}", e);
			ok(create_error_response(e))
		}))
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::new(ok(create_ok_response("{}")))
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
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		)
		.body(format!("{}", e).into())
		.unwrap()
}

fn create_ok_response(json: &str) -> Response<Body> {
	Response::builder()
		.status(StatusCode::OK)
		.header("access-control-allow-origin", "*")
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		)
		.body(json.to_string().into())
		.unwrap()
}

fn response<T: Into<Body>>(status: StatusCode, text: T) -> Response<Body> {
	Response::builder()
		.status(status)
		.header("access-control-allow-origin", "*")
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		)
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

fn parse_body<T>(req: Request<Body>) -> Box<dyn Future<Item = T, Error = Error> + Send>
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
