// Copyright 2021 The Grin Developers
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
use crate::core::ser::{self, DeserializationMode, ProtocolVersion};
use crate::pool::{self, BlockChain, PoolAdapter, PoolEntry};
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::util;
use crate::util::RwLock;
use crate::web::*;
use hyper::{Body, Request, StatusCode};
use std::sync::Weak;

/// Get basic information about the transaction pool.
/// GET /v1/pool
pub struct PoolInfoHandler<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
}

impl<B, P> Handler for PoolInfoHandler<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		let pool_arc = w_fut!(&self.tx_pool);
		let pool = pool_arc.read();

		json_response(&PoolInfo {
			pool_size: pool.total_size(),
		})
	}
}

pub struct PoolHandler<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
}

impl<B, P> PoolHandler<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
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
			.map_err(|e| Error::Internal(format!("Failed to get chain head: {}", e)))?;
		tx_pool
			.add_to_pool(source, tx, !fluff.unwrap_or(false), &header)
			.map_err(|e| Error::Internal(format!("Failed to update pool: {}", e)))?;
		Ok(())
	}
}
/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
struct TxWrapper {
	tx_hex: String,
}

/// Push new transaction to our local transaction pool.
/// POST /v1/pool/push_tx
pub struct PoolPushHandler<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
}

async fn update_pool<B, P>(
	pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
	req: Request<Body>,
) -> Result<(), Error>
where
	B: BlockChain,
	P: PoolAdapter,
{
	let pool = w(&pool)?;
	let params = QueryParams::from(req.uri().query());
	let fluff = params.get("fluff").is_some();

	let wrapper: TxWrapper = parse_body(req).await?;
	let tx_bin = util::from_hex(&wrapper.tx_hex)
		.map_err(|e| Error::RequestError(format!("Bad request: {}", e)))?;

	// All wallet api interaction explicitly uses protocol version 1 for now.
	let version = ProtocolVersion(1);
	let tx: Transaction =
		ser::deserialize(&mut &tx_bin[..], version, DeserializationMode::default())
			.map_err(|e| Error::RequestError(format!("Bad request: {}", e)))?;

	let source = pool::TxSource::PushApi;
	info!(
		"Pushing transaction {} to pool (inputs: {}, outputs: {}, kernels: {})",
		tx.hash(),
		tx.inputs().len(),
		tx.outputs().len(),
		tx.kernels().len(),
	);

	//  Push to tx pool.
	let mut tx_pool = pool.write();
	let header = tx_pool
		.blockchain
		.chain_head()
		.map_err(|e| Error::Internal(format!("Failed to get chain head: {}", e)))?;
	tx_pool
		.add_to_pool(source, tx, !fluff, &header)
		.map_err(|e| Error::Internal(format!("Failed to update pool: {}", e)))?;
	Ok(())
}

impl<B, P> Handler for PoolPushHandler<B, P>
where
	B: BlockChain + 'static,
	P: PoolAdapter + 'static,
{
	fn post(&self, req: Request<Body>) -> ResponseFuture {
		let pool = self.tx_pool.clone();
		Box::pin(async move {
			let res = match update_pool(pool, req).await {
				Ok(_) => just_response(StatusCode::OK, ""),
				Err(e) => {
					just_response(StatusCode::INTERNAL_SERVER_ERROR, format!("failed: {}", e))
				}
			};
			Ok(res)
		})
	}
}
