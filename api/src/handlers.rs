// Copyright 2020 The Grin Developers
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

pub mod blocks_api;
pub mod chain_api;
pub mod peers_api;
pub mod pool_api;
pub mod server_api;
pub mod transactions_api;
pub mod utils;
pub mod version_api;

use self::blocks_api::BlockHandler;
use self::blocks_api::HeaderHandler;
use self::chain_api::ChainCompactHandler;
use self::chain_api::ChainHandler;
use self::chain_api::ChainValidationHandler;
use self::chain_api::KernelHandler;
use self::chain_api::OutputHandler;
use self::peers_api::PeerHandler;
use self::peers_api::PeersAllHandler;
use self::peers_api::PeersConnectedHandler;
use self::pool_api::PoolInfoHandler;
use self::pool_api::PoolPushHandler;
use self::server_api::IndexHandler;
use self::server_api::StatusHandler;
use self::transactions_api::TxHashSetHandler;
use self::version_api::VersionHandler;
use crate::auth::{
	BasicAuthMiddleware, BasicAuthURIMiddleware, GRIN_BASIC_REALM, GRIN_FOREIGN_BASIC_REALM,
};
use crate::chain;
use crate::chain::{Chain, SyncState};
use crate::core::core::verifier_cache::VerifierCache;
use crate::foreign::Foreign;
use crate::foreign_rpc::ForeignRpc;
use crate::owner::Owner;
use crate::owner_rpc::OwnerRpc;
use crate::p2p;
use crate::pool;
use crate::pool::{BlockChain, PoolAdapter};
use crate::rest::{ApiServer, Error, TLSConfig};
use crate::router::ResponseFuture;
use crate::router::{Router, RouterError};
use crate::util::to_base64;
use crate::util::RwLock;
use crate::web::*;
use easy_jsonrpc_mw::{Handler, MaybeReply};
use hyper::{Body, Request, Response, StatusCode};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};

/// Listener version, providing same API but listening for requests on a
/// port and wrapping the calls
pub fn node_apis<B, P, V>(
	addr: &str,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<B, P, V>>>,
	peers: Arc<p2p::Peers>,
	sync_state: Arc<chain::SyncState>,
	api_secret: Option<String>,
	foreign_api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
) -> Result<(), Error>
where
	B: BlockChain + 'static,
	P: PoolAdapter + 'static,
	V: VerifierCache + 'static,
{
	// Manually build router when getting rid of v1
	//let mut router = Router::new();
	let mut router = build_router(
		chain.clone(),
		tx_pool.clone(),
		peers.clone(),
		sync_state.clone(),
	)
	.expect("unable to build API router");

	// Add basic auth to v1 API and owner v2 API
	if let Some(api_secret) = api_secret {
		let api_basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret));
		let basic_auth_middleware = Arc::new(BasicAuthMiddleware::new(
			api_basic_auth,
			&GRIN_BASIC_REALM,
			Some("/v2/foreign".into()),
		));
		router.add_middleware(basic_auth_middleware);
	}

	let api_handler_v2 = OwnerAPIHandlerV2::new(
		Arc::downgrade(&chain),
		Arc::downgrade(&peers),
		Arc::downgrade(&sync_state),
	);
	router.add_route("/v2/owner", Arc::new(api_handler_v2))?;

	// Add basic auth to v2 foreign API only
	if let Some(api_secret) = foreign_api_secret {
		let api_basic_auth =
			"Basic ".to_string() + &to_base64(&("grin:".to_string() + &api_secret));
		let basic_auth_middleware = Arc::new(BasicAuthURIMiddleware::new(
			api_basic_auth,
			&GRIN_FOREIGN_BASIC_REALM,
			"/v2/foreign".into(),
		));
		router.add_middleware(basic_auth_middleware);
	}

	let api_handler_v2 = ForeignAPIHandlerV2::new(
		Arc::downgrade(&chain),
		Arc::downgrade(&tx_pool),
		Arc::downgrade(&sync_state),
	);
	router.add_route("/v2/foreign", Arc::new(api_handler_v2))?;

	let mut apis = ApiServer::new();
	warn!("Starting HTTP Node APIs server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let api_thread = apis.start(socket_addr, router, tls_config);

	warn!("HTTP Node listener started.");

	match api_thread {
		Ok(_) => Ok(()),
		Err(e) => {
			error!("HTTP API server failed to start. Err: {}", e);
			Err(e)
		}
	}
}

/// V2 API Handler/Wrapper for owner functions
pub struct OwnerAPIHandlerV2 {
	pub chain: Weak<Chain>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl OwnerAPIHandlerV2 {
	/// Create a new owner API handler for GET methods
	pub fn new(chain: Weak<Chain>, peers: Weak<p2p::Peers>, sync_state: Weak<SyncState>) -> Self {
		OwnerAPIHandlerV2 {
			chain,
			peers,
			sync_state,
		}
	}
}

impl crate::router::Handler for OwnerAPIHandlerV2 {
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let api = Owner::new(
			self.chain.clone(),
			self.peers.clone(),
			self.sync_state.clone(),
		);

		Box::pin(async move {
			match parse_body(req).await {
				Ok(val) => {
					let owner_api = &api as &dyn OwnerRpc;
					let res = match owner_api.handle_request(val) {
						MaybeReply::Reply(r) => r,
						MaybeReply::DontReply => {
							// Since it's http, we need to return something. We return [] because jsonrpc
							// clients will parse it as an empty batch response.
							serde_json::json!([])
						}
					};
					Ok(json_response_pretty(&res))
				}
				Err(e) => {
					error!("Request Error: {:?}", e);
					Ok(create_error_response(e))
				}
			}
		})
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::pin(async { Ok(create_ok_response("{}")) })
	}
}

/// V2 API Handler/Wrapper for foreign functions
pub struct ForeignAPIHandlerV2<B, P, V>
where
	B: BlockChain,
	P: PoolAdapter,
	V: VerifierCache + 'static,
{
	pub chain: Weak<Chain>,
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P, V>>>,
	pub sync_state: Weak<SyncState>,
}

impl<B, P, V> ForeignAPIHandlerV2<B, P, V>
where
	B: BlockChain,
	P: PoolAdapter,
	V: VerifierCache + 'static,
{
	/// Create a new foreign API handler for GET methods
	pub fn new(
		chain: Weak<Chain>,
		tx_pool: Weak<RwLock<pool::TransactionPool<B, P, V>>>,
		sync_state: Weak<SyncState>,
	) -> Self {
		ForeignAPIHandlerV2 {
			chain,
			tx_pool,
			sync_state,
		}
	}
}

impl<B, P, V> crate::router::Handler for ForeignAPIHandlerV2<B, P, V>
where
	B: BlockChain + 'static,
	P: PoolAdapter + 'static,
	V: VerifierCache + 'static,
{
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let api = Foreign::new(
			self.chain.clone(),
			self.tx_pool.clone(),
			self.sync_state.clone(),
		);

		Box::pin(async move {
			match parse_body(req).await {
				Ok(val) => {
					let foreign_api = &api as &dyn ForeignRpc;
					let res = match foreign_api.handle_request(val) {
						MaybeReply::Reply(r) => r,
						MaybeReply::DontReply => {
							// Since it's http, we need to return something. We return [] because jsonrpc
							// clients will parse it as an empty batch response.
							serde_json::json!([])
						}
					};
					Ok(json_response_pretty(&res))
				}
				Err(e) => {
					error!("Request Error: {:?}", e);
					Ok(create_error_response(e))
				}
			}
		})
	}

	fn options(&self, _req: Request<Body>) -> ResponseFuture {
		Box::pin(async { Ok(create_ok_response("{}")) })
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
		.header(hyper::header::CONTENT_TYPE, "application/json")
		.body(json.to_string().into())
		.unwrap()
}

/// Build a new hyper Response with the status code and body provided.
///
/// Whenever the status code is `StatusCode::OK` the text parameter should be
/// valid JSON as the content type header will be set to `application/json'
fn response<T: Into<Body>>(status: StatusCode, text: T) -> Response<Body> {
	let mut builder = Response::builder();

	builder = builder
		.status(status)
		.header("access-control-allow-origin", "*")
		.header(
			"access-control-allow-headers",
			"Content-Type, Authorization",
		);

	if status == StatusCode::OK {
		builder = builder.header(hyper::header::CONTENT_TYPE, "application/json");
	}

	builder.body(text.into()).unwrap()
}

// Legacy V1 router
#[deprecated(
	since = "4.0.0",
	note = "The V1 Node API will be removed in grin 5.0.0. Please migrate to the V2 API as soon as possible."
)]
pub fn build_router<B, P, V>(
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<B, P, V>>>,
	peers: Arc<p2p::Peers>,
	sync_state: Arc<chain::SyncState>,
) -> Result<Router, RouterError>
where
	B: BlockChain + 'static,
	P: PoolAdapter + 'static,
	V: VerifierCache + 'static,
{
	let route_list = vec![
		"get blocks".to_string(),
		"get headers".to_string(),
		"get chain".to_string(),
		"post chain/compact".to_string(),
		"get chain/validate".to_string(),
		"get chain/kernels/xxx?min_height=yyy&max_height=zzz".to_string(),
		"get chain/outputs/byids?id=xxx,yyy,zzz".to_string(),
		"get chain/outputs/byheight?start_height=101&end_height=200".to_string(),
		"get status".to_string(),
		"get txhashset/roots".to_string(),
		"get txhashset/lastoutputs?n=10".to_string(),
		"get txhashset/lastrangeproofs".to_string(),
		"get txhashset/lastkernels".to_string(),
		"get txhashset/outputs?start_index=1&max=100".to_string(),
		"get txhashset/merkleproof?n=1".to_string(),
		"get pool".to_string(),
		"post pool/push_tx".to_string(),
		"post peers/a.b.c.d:p/ban".to_string(),
		"post peers/a.b.c.d:p/unban".to_string(),
		"get peers/all".to_string(),
		"get peers/connected".to_string(),
		"get peers/a.b.c.d".to_string(),
		"get version".to_string(),
	];
	let index_handler = IndexHandler { list: route_list };

	let output_handler = OutputHandler {
		chain: Arc::downgrade(&chain),
	};
	let kernel_handler = KernelHandler {
		chain: Arc::downgrade(&chain),
	};
	let block_handler = BlockHandler {
		chain: Arc::downgrade(&chain),
	};
	let header_handler = HeaderHandler {
		chain: Arc::downgrade(&chain),
	};
	let chain_tip_handler = ChainHandler {
		chain: Arc::downgrade(&chain),
	};
	let chain_compact_handler = ChainCompactHandler {
		chain: Arc::downgrade(&chain),
	};
	let chain_validation_handler = ChainValidationHandler {
		chain: Arc::downgrade(&chain),
	};
	let status_handler = StatusHandler {
		chain: Arc::downgrade(&chain),
		peers: Arc::downgrade(&peers),
		sync_state: Arc::downgrade(&sync_state),
	};
	let txhashset_handler = TxHashSetHandler {
		chain: Arc::downgrade(&chain),
	};
	let pool_info_handler = PoolInfoHandler {
		tx_pool: Arc::downgrade(&tx_pool),
	};
	let pool_push_handler = PoolPushHandler {
		tx_pool: Arc::downgrade(&tx_pool),
	};
	let peers_all_handler = PeersAllHandler {
		peers: Arc::downgrade(&peers),
	};
	let peers_connected_handler = PeersConnectedHandler {
		peers: Arc::downgrade(&peers),
	};
	let peer_handler = PeerHandler {
		peers: Arc::downgrade(&peers),
	};
	let version_handler = VersionHandler {
		chain: Arc::downgrade(&chain),
	};

	let mut router = Router::new();

	router.add_route("/v1/", Arc::new(index_handler))?;
	router.add_route("/v1/blocks/*", Arc::new(block_handler))?;
	router.add_route("/v1/headers/*", Arc::new(header_handler))?;
	router.add_route("/v1/chain", Arc::new(chain_tip_handler))?;
	router.add_route("/v1/chain/outputs/*", Arc::new(output_handler))?;
	router.add_route("/v1/chain/kernels/*", Arc::new(kernel_handler))?;
	router.add_route("/v1/chain/compact", Arc::new(chain_compact_handler))?;
	router.add_route("/v1/chain/validate", Arc::new(chain_validation_handler))?;
	router.add_route("/v1/txhashset/*", Arc::new(txhashset_handler))?;
	router.add_route("/v1/status", Arc::new(status_handler))?;
	router.add_route("/v1/pool", Arc::new(pool_info_handler))?;
	router.add_route("/v1/pool/push_tx", Arc::new(pool_push_handler))?;
	router.add_route("/v1/peers/all", Arc::new(peers_all_handler))?;
	router.add_route("/v1/peers/connected", Arc::new(peers_connected_handler))?;
	router.add_route("/v1/peers/**", Arc::new(peer_handler))?;
	router.add_route("/v1/version", Arc::new(version_handler))?;
	Ok(router)
}
