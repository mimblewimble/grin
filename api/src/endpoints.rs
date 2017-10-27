// Copyright 2016 The Grin Developers
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


use std::sync::{Arc, RwLock};
use std::thread;

use chain;
use core::core::Transaction;
use core::ser;
use pool;
use handlers::{UtxoHandler, ChainHandler, SumTreeHandler};
use rest::*;
use types::*;
use util;
use util::LOGGER;

/// ApiEndpoint implementation for the transaction pool, to check its status
/// and size as well as push new transactions.
#[derive(Clone)]
pub struct PoolApi<T> {
	tx_pool: Arc<RwLock<pool::TransactionPool<T>>>,
}

impl<T> ApiEndpoint for PoolApi<T>
where
	T: pool::BlockChain + Clone + Send + Sync + 'static,
{
	type ID = String;
	type T = PoolInfo;
	type OP_IN = TxWrapper;
	type OP_OUT = ();

	fn operations(&self) -> Vec<Operation> {
		vec![Operation::Get, Operation::Custom("push".to_string())]
	}

	fn get(&self, _: String) -> ApiResult<PoolInfo> {
		let pool = self.tx_pool.read().unwrap();
		Ok(PoolInfo {
			pool_size: pool.pool_size(),
			orphans_size: pool.orphans_size(),
			total_size: pool.total_size(),
		})
	}

	fn operation(&self, _: String, input: TxWrapper) -> ApiResult<()> {
		let tx_bin = util::from_hex(input.tx_hex).map_err(|_| {
			Error::Argument(format!("Invalid hex in transaction wrapper."))
		})?;

		let tx: Transaction = ser::deserialize(&mut &tx_bin[..]).map_err(|_| {
			Error::Argument(
				"Could not deserialize transaction, invalid format.".to_string(),
			)
		})?;

		let source = pool::TxSource {
			debug_name: "push-api".to_string(),
			identifier: "?.?.?.?".to_string(),
		};
		info!(
			LOGGER,
			"Pushing transaction with {} inputs and {} outputs to pool.",
			tx.inputs.len(),
			tx.outputs.len()
		);
		self.tx_pool
			.write()
			.unwrap()
			.add_to_memory_pool(source, tx)
			.map_err(|e| {
				Error::Internal(format!("Addition to transaction pool failed: {:?}", e))
			})?;

		Ok(())
	}
}

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	tx_hex: String,
}

/// Start all server REST APIs. Just register all of them on a ApiServer
/// instance and runs the corresponding HTTP server.
pub fn start_rest_apis<T>(
	addr: String,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<T>>>,
) where
	T: pool::BlockChain + Clone + Send + Sync + 'static,
{

	thread::spawn(move || {
		let mut apis = ApiServer::new("/v1".to_string());
		apis.register_endpoint("/pool".to_string(), PoolApi {tx_pool: tx_pool});

		// register a nested router at "/v2" for flexibility
		// so we can experiment with raw iron handlers
		let utxo_handler = UtxoHandler {chain: chain.clone()};
		let chain_tip_handler = ChainHandler {chain: chain.clone()};
		let sumtree_handler = SumTreeHandler {chain: chain.clone()};
		let router = router!(
			chain_tip: get "/chain" => chain_tip_handler,
			chain_utxos: get "/chain/utxos" => utxo_handler,
			sumtree_roots: get "/sumtrees/*" => sumtree_handler,
		);
		apis.register_handler("/v2", router);

		apis.start(&addr[..]).unwrap_or_else(|e| {
			error!(LOGGER, "Failed to start API HTTP server: {}.", e);
		});
	});
}
