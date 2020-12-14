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
use crate::core::core::verifier_cache::VerifierCache;
use crate::core::core::Transaction;
use crate::error::*;
use crate::pool::{self, BlockChain, PoolAdapter, PoolEntry};
use crate::util::RwLock;
use failure::ResultExt;
use std::sync::Weak;

/// Get basic information about the transaction pool.
pub struct PoolHandler<B, P, V>
where
	B: BlockChain,
	P: PoolAdapter,
	V: VerifierCache + 'static,
{
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P, V>>>,
}

impl<B, P, V> PoolHandler<B, P, V>
where
	B: BlockChain,
	P: PoolAdapter,
	V: VerifierCache + 'static,
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
			.context(ErrorKind::Internal("Failed to get chain head".to_owned()))?;
		tx_pool
			.add_to_pool(source, tx, !fluff.unwrap_or(false), &header)
			.context(ErrorKind::Internal("Failed to update pool".to_owned()))?;
		Ok(())
	}
}
