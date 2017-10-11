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
use rest::*;
use types::*;
use secp::pedersen::Commitment;
use util;

/// ApiEndpoint implementation for the blockchain. Exposes the current chain
/// state as a simple JSON object.
#[derive(Clone)]
pub struct ChainApi {
	/// data store access
	chain: Arc<chain::Chain>,
}

impl ApiEndpoint for ChainApi {
	type ID = String;
	type T = Tip;
	type OP_IN = ();
	type OP_OUT = ();

	fn operations(&self) -> Vec<Operation> {
		vec![Operation::Get]
	}

	fn get(&self, _: String) -> ApiResult<Tip> {
		match self.chain.head() {
			Ok(tip) => Ok(Tip::from_tip(tip)),
			Err(e) => Err(Error::Internal(format!("{:?}", e))),
		}
	}
}

/// ApiEndpoint implementation for outputs that have been included in the chain.
#[derive(Clone)]
pub struct OutputApi {
	/// data store access
	chain: Arc<chain::Chain>,
}

impl ApiEndpoint for OutputApi {
	type ID = String;
	type T = Output;
	type OP_IN = ();
	type OP_OUT = ();

	fn operations(&self) -> Vec<Operation> {
		vec![Operation::Get]
	}

	fn get(&self, id: String) -> ApiResult<Output> {
		debug!("GET output {}", id);
		let c = util::from_hex(id.clone()).map_err(|_| {
			Error::Argument(format!("Not a valid commitment: {}", id))
		})?;
		let commit = Commitment::from_vec(c);

		let out = self.chain.get_unspent(&commit).map_err(|_| Error::NotFound)?;

		let header = self.chain
			.get_block_header_by_output_commit(&commit)
			.map_err(|_| Error::NotFound)?;

		Ok(Output::from_output(&out, &header))
	}
}

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
		apis.register_endpoint("/chain".to_string(), ChainApi { chain: chain.clone() });
		apis.register_endpoint(
			"/chain/utxo".to_string(),
			OutputApi { chain: chain.clone() },
		);
		apis.register_endpoint("/pool".to_string(), PoolApi { tx_pool: tx_pool });

		apis.start(&addr[..]).unwrap_or_else(|e| {
			error!("Failed to start API HTTP server: {}.", e);
		});
	});
}
