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

mod blocks_api;
mod chain_api;
mod peers_api;
mod pool_api;
mod server_api;
mod transactions_api;
mod utils;
mod version_api;

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
use self::server_api::KernelDownloadHandler;
use self::server_api::StatusHandler;
use self::transactions_api::TxHashSetHandler;
use self::version_api::VersionHandler;
use crate::auth::{BasicAuthMiddleware, GRIN_BASIC_REALM};
use crate::chain;
use crate::p2p;
use crate::pool;
use crate::rest::*;
use crate::router::{Router, RouterError};
use crate::util;
use crate::util::RwLock;
use std::net::SocketAddr;
use std::sync::Arc;

/// Start all server HTTP handlers. Register all of them with Router
/// and runs the corresponding HTTP server.
///
/// Hyper currently has a bug that prevents clean shutdown. In order
/// to avoid having references kept forever by handlers, we only pass
/// weak references. Note that this likely means a crash if the handlers are
/// used after a server shutdown (which should normally never happen,
/// except during tests).
pub fn start_rest_apis(
	addr: String,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	peers: Arc<p2p::Peers>,
	api_secret: Option<String>,
	tls_config: Option<TLSConfig>,
) -> bool {
	let mut apis = ApiServer::new();
	let mut router = build_router(chain, tx_pool, peers).expect("unable to build API router");
	if let Some(api_secret) = api_secret {
		let api_basic_auth = format!("Basic {}", util::to_base64(&format!("grin:{}", api_secret)));
		let basic_auth_middleware = Arc::new(BasicAuthMiddleware::new(
			api_basic_auth,
			&GRIN_BASIC_REALM,
			None,
		));
		router.add_middleware(basic_auth_middleware);
	}

	info!("Starting HTTP API server at {}.", addr);
	let socket_addr: SocketAddr = addr.parse().expect("unable to parse socket address");
	let res = apis.start(socket_addr, router, tls_config);
	match res {
		Ok(_) => true,
		Err(e) => {
			error!("HTTP API server failed to start. Err: {}", e);
			false
		}
	}
}

pub fn build_router(
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	peers: Arc<p2p::Peers>,
) -> Result<Router, RouterError> {
	let route_list = vec![
		"get blocks".to_string(),
		"get headers".to_string(),
		"get chain".to_string(),
		"post chain/compact".to_string(),
		"get chain/validate".to_string(),
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
	};
	let kernel_download_handler = KernelDownloadHandler {
		peers: Arc::downgrade(&peers),
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
	router.add_route("/v1/kerneldownload", Arc::new(kernel_download_handler))?;
	router.add_route("/v1/pool", Arc::new(pool_info_handler))?;
	router.add_route("/v1/pool/push_tx", Arc::new(pool_push_handler))?;
	router.add_route("/v1/peers/all", Arc::new(peers_all_handler))?;
	router.add_route("/v1/peers/connected", Arc::new(peers_connected_handler))?;
	router.add_route("/v1/peers/**", Arc::new(peer_handler))?;
	router.add_route("/v1/version", Arc::new(version_handler))?;
	Ok(router)
}
