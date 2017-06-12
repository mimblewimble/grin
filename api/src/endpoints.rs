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

// pub struct HashID(pub [u8; 32]);
//
// impl FromStr for HashId {
//   type Err = ;
//
//   fn from_str(s: &str) -> Result<HashId, > {
//   }
// }

use std::sync::{Arc, Mutex, RwLock};
use std::thread;

use core::core::{Transaction, Output};
use core::core::hash::Hash;
use core::ser;
use chain::{self, Tip};
use pool;
use rest::*;
use secp::pedersen::Commitment;
use util;

/// ApiEndpoint implementation for the blockchain. Exposes the current chain
/// state as a simple JSON object.
#[derive(Clone)]
pub struct ChainApi {
	/// data store access
	chain_store: Arc<chain::ChainStore>,
}

impl ApiEndpoint for ChainApi {
	type ID = String;
	type T = Tip;
	type OP_IN = ();
	type OP_OUT = ();

	fn operations(&self) -> Vec<Operation> {
		vec![Operation::Get]
	}

	fn get(&self, id: String) -> ApiResult<Tip> {
		self.chain_store.head().map_err(|e| Error::Internal(e.to_string()))
	}
}

/// ApiEndpoint implementation for outputs that have been included in the chain.
#[derive(Clone)]
pub struct OutputApi {
	/// data store access
	chain_store: Arc<chain::ChainStore>,
	chain_head: Arc<Mutex<chain::Tip>>,
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
		let c = util::from_hex(id.clone()).map_err(|e| Error::Argument(format!("Not a valid commitment: {}", id)))?;
		let commitment = Commitment::from_vec(c);

		// TODO use an actual UTXO tree
		// in the meantime doing it the *very* expensive way:
		//   1. check the output exists
		//   2. run the chain back from the head to check it hasn't been spent
		if let Ok(out) = self.chain_store.get_output_by_commit(&commitment) {
			let mut block_h: Hash;
			{
				let chain_head = self.chain_head.clone();
				let head = chain_head.lock().unwrap();
				block_h = head.last_block_h;
			}
			loop {
				let b = self.chain_store.get_block(&block_h)?;
				for input in b.inputs {
					if input.commitment() == commitment {
						return Err(Error::NotFound);
					}
				}
				if b.header.height == 1 {
					return Ok(out);
				} else {
					block_h = b.header.previous;
				}
			}
		}
		Err(Error::NotFound)
	}
}

/// ApiEndpoint implementation for the transaction pool, to check its status
/// and size as well as push new transactions.
#[derive(Clone)]
pub struct PoolApi<T> {
	tx_pool: Arc<RwLock<pool::TransactionPool<T>>>,
}

#[derive(Serialize, Deserialize)]
struct PoolInfo {
	pool_size: usize,
	orphans_size: usize,
	total_size: usize,
}

impl<T> ApiEndpoint for PoolApi<T>
    where T: pool::BlockChain + Clone + Send + Sync + 'static
{
	type ID = String;
	type T = PoolInfo;
	type OP_IN = TxWrapper;
	type OP_OUT = ();

	fn operations(&self) -> Vec<Operation> {
		vec![Operation::Get, Operation::Custom("push".to_string())]
	}

	fn get(&self, id: String) -> ApiResult<PoolInfo> {
		let pool = self.tx_pool.read().unwrap();
		Ok(PoolInfo {
			pool_size: pool.pool_size(),
			orphans_size: pool.orphans_size(),
			total_size: pool.total_size(),
		})
	}

	fn operation(&self, op: String, input: TxWrapper) -> ApiResult<()> {
		let tx_bin = util::from_hex(input.tx_hex)
      .map_err(|e| Error::Argument(format!("Invalid hex in transaction wrapper.")))?;

		let tx: Transaction = ser::deserialize(&mut &tx_bin[..]).map_err(|_| {
				Error::Argument("Could not deserialize transaction, invalid format.".to_string())
			})?;

		let source = pool::TxSource {
			debug_name: "push-api".to_string(),
			identifier: "?.?.?.?".to_string(),
		};
		debug!("Pushing transaction with {} inputs and {} outputs to pool.",
		       tx.inputs.len(),
		       tx.outputs.len());
		self.tx_pool
			.write()
			.unwrap()
			.add_to_memory_pool(source, tx)
			.map_err(|e| Error::Internal(format!("Addition to transaction pool failed: {:?}", e)))?;
		Ok(())
	}
}

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
struct TxWrapper {
	tx_hex: String,
}

/// Start all server REST APIs. Just register all of them on a ApiServer
/// instance and runs the corresponding HTTP server.
pub fn start_rest_apis<T>(addr: String,
                          chain_store: Arc<chain::ChainStore>,
                          chain_head: Arc<Mutex<chain::Tip>>,
                          tx_pool: Arc<RwLock<pool::TransactionPool<T>>>)
	where T: pool::BlockChain + Clone + Send + Sync + 'static
{

	thread::spawn(move || {
		let mut apis = ApiServer::new("/v1".to_string());
		apis.register_endpoint("/chain".to_string(),
		                       ChainApi { chain_store: chain_store.clone() });
		apis.register_endpoint("/chain/utxo".to_string(),
		                       OutputApi {
			                       chain_store: chain_store.clone(),
			                       chain_head: chain_head.clone(),
		                       });
		apis.register_endpoint("/pool".to_string(), PoolApi { tx_pool: tx_pool });

		apis.start(&addr[..]).unwrap_or_else(|e| {
			error!("Failed to start API HTTP server: {}.", e);
		});
	});
}
