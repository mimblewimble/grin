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

use super::utils::w;
use crate::core::core::hash::Hashed;
use crate::core::core::Transaction;
use crate::core::ser::{self, ProtocolVersion};
use crate::pool::{self, PoolEntry};
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::util;
use crate::util::RwLock;
use crate::web::*;
use failure::ResultExt;
use futures::future::{err, ok};
use futures::Future;
use hyper::{Body, Request, StatusCode};
use std::sync::Weak;

/// Get basic information about the transaction pool.
/// GET /v1/pool
pub struct PoolInfoHandler {
	pub tx_pool: Weak<RwLock<pool::TransactionPool>>,
}

impl Handler for PoolInfoHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let pool_arc = w_fut!(&self.tx_pool);
		let pool = pool_arc.read();

		json_response(&PoolInfo {
			pool_size: pool.total_size(),
		})
	}
}

pub struct PoolHandler {
	pub tx_pool: Weak<RwLock<pool::TransactionPool>>,
}

impl PoolHandler {
	pub fn get_pool_size(&self) -> Result<usize, Error> {
		let pool_arc = w(&self.tx_pool)?;
		let pool = pool_arc.read();
		Ok(pool.total_size())
	}
	pub fn get_stempool_size(&self) -> Result<usize, Error> {
		let pool_arc = w(&self.tx_pool)?;
		let pool = pool_arc.read();
		Ok(pool.stempool.size())
	}
	pub fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, Error> {
		// will only read from txpool
		let pool_arc = w(&self.tx_pool)?;
		let txpool = pool_arc.read();
		Ok(txpool.txpool.entries.clone())
	}
	pub fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), Error> {
		let pool_arc = w(&self.tx_pool)?;
		let source = pool::TxSource::PushApi;
		info!(
			"Pushing transaction {} to pool (inputs: {}, outputs: {}, kernels: {})",
			tx.hash(),
			tx.inputs().len(),
			tx.outputs().len(),
			tx.kernels().len(),
		);

		//  Push to tx pool.
		let mut tx_pool = pool_arc.write();
		let header = tx_pool
			.blockchain
			.chain_head()
			.context(ErrorKind::Internal("Failed to get chain head".to_owned()))?;
		let res = tx_pool
			.add_to_pool(source, tx, !fluff.unwrap_or(false), &header)
			.context(ErrorKind::Internal("Failed to update pool".to_owned()))?;
		Ok(res)
	}
}
/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
struct TxWrapper {
	tx_hex: String,
}

/// Push new transaction to our local transaction pool.
/// POST /v1/pool/push_tx
pub struct PoolPushHandler {
	pub tx_pool: Weak<RwLock<pool::TransactionPool>>,
}

impl PoolPushHandler {
	fn update_pool(&self, req: Request<Body>) -> Box<dyn Future<Item = (), Error = Error> + Send> {
		let params = QueryParams::from(req.uri().query());

		let fluff = params.get("fluff").is_some();
		let pool_arc = match w(&self.tx_pool) {
			Ok(p) => p,
			Err(e) => return Box::new(err(e)),
		};

		Box::new(
			parse_body(req)
				.and_then(move |wrapper: TxWrapper| {
					util::from_hex(wrapper.tx_hex)
						.map_err(|e| ErrorKind::RequestError(format!("Bad request: {}", e)).into())
				})
				.and_then(move |tx_bin| {
					// All wallet api interaction explicitly uses protocol version 1 for now.
					let version = ProtocolVersion(1);

					ser::deserialize(&mut &tx_bin[..], version)
						.map_err(|e| ErrorKind::RequestError(format!("Bad request: {}", e)).into())
				})
				.and_then(move |tx: Transaction| {
					let source = pool::TxSource::PushApi;
					info!(
						"Pushing transaction {} to pool (inputs: {}, outputs: {}, kernels: {})",
						tx.hash(),
						tx.inputs().len(),
						tx.outputs().len(),
						tx.kernels().len(),
					);

					//  Push to tx pool.
					let mut tx_pool = pool_arc.write();
					let header = tx_pool
						.blockchain
						.chain_head()
						.context(ErrorKind::Internal("Failed to get chain head".to_owned()))?;
					let res = tx_pool
						.add_to_pool(source, tx, !fluff, &header)
						.context(ErrorKind::Internal("Failed to update pool".to_owned()))?;
					Ok(res)
				}),
		)
	}
}

impl Handler for PoolPushHandler {
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		Box::new(
			self.update_pool(req)
				.and_then(|_| ok(just_response(StatusCode::OK, "")))
				.or_else(|e| {
					ok(just_response(
						StatusCode::INTERNAL_SERVER_ERROR,
						format!("failed: {}", e),
					))
				}),
		)
	}
}
