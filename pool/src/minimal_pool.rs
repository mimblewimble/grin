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

//! A minimal (EXPERIMENTAL) transaction pool implementation

use std::collections::HashMap;
use std::sync::Arc;
use core::core::hash::{Hash, Hashed};
use core::core::{Committed, Transaction};
use types::*;
use util::{secp_static, static_secp_instance};
use util::secp::pedersen::Commitment;

/// Sums of the pool.
#[derive(Debug, Clone)]
pub struct TxPoolSums {
	/// Sum of all input|output commitments of all pool txs.
	pub output_sum: Commitment,
	/// Sum of kernel excesses of all pool txs.
	pub kernel_sum: Commitment,
	/// Sum of kernel offsets of all pool txs.
	pub kernel_offset: Commitment,
	/// Sum of fees of all pool txs.
	pub fee: u64,
}

impl Default for TxPoolSums {
	fn default() -> TxPoolSums {
		let zero_commit = secp_static::commit_to_zero_value();
		TxPoolSums {
			output_sum: zero_commit.clone(),
			kernel_sum: zero_commit.clone(),
			kernel_offset: zero_commit.clone(),
			fee: 0,
		}
	}
}

impl Committed for TxPoolSums {
	fn inputs_committed(&self) -> Vec<Commitment> {
		vec![]
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		vec![self.output_sum]
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		vec![self.kernel_sum]
	}
}

/// A minimal (EXPERIMENTAL) transaction pool implementation
pub struct MinimalTxPool<T> {
	/// Pool Config
	pub config: PoolConfig,
	/// Transaction in the pool keyed by hash
	pub transactions: HashMap<Hash, Box<Transaction>>,
	/// Transaction hashes in the order they were added to the pool
	pub tx_insert_order: Vec<Hash>,
	/// The blockchain
	pub blockchain: Arc<T>,
	/// The pool adapter
	pub adapter: Arc<PoolAdapter>,
}

impl<T> Committed for MinimalTxPool<T> {
	fn inputs_committed(&self) -> Vec<Commitment> {
		self.transactions
			.values()
			.flat_map(|x| &x.inputs)
			.map(|x| x.commitment())
			.collect()
	}

	fn outputs_committed(&self) -> Vec<Commitment> {
		self.transactions
			.values()
			.flat_map(|x| &x.outputs)
			.map(|x| x.commitment())
			.collect()
	}

	fn kernels_committed(&self) -> Vec<Commitment> {
		self.transactions
			.values()
			.flat_map(|x| &x.kernels)
			.map(|x| x.excess())
			.collect()
	}
}

impl<T> MinimalTxPool<T>
where
	T: BlockChain,
{
	/// Create a new transaction pool
	pub fn new(config: PoolConfig, chain: Arc<T>, adapter: Arc<PoolAdapter>) -> MinimalTxPool<T> {
		MinimalTxPool {
			config: config,
			pool_sums: TxPoolSums::default(),
			transactions: HashMap::new(),
			tx_insert_order: Vec::new(),
			blockchain: chain,
			adapter: adapter,
		}
	}

	/// Add a new transacton to the pool.
	pub fn add_to_memory_pool(
		&mut self,
		_: TxSource,
		tx: Transaction,
		_stem: bool,
	) -> Result<(), PoolError> {
		// Do we have the capacity to accept this transaction?
		self.is_acceptable(&tx)?;

		// Making sure the transaction is valid before anything else.
		tx.validate().map_err(|e| PoolError::InvalidTx(e))?;

		let zero_commit = secp_static::commit_to_zero_value();

		// tx.sum_kernel_excesses(, None);

		// taking the -
		// * current block_sums from the chain
		// * current kernel_offset from the header
		// we can then sum the tx into these to check validity

		// tx implements Committed
		// tx_pool implements Committed

		let header = self.blockchain.head_header()?;
		let block_sums = self.blockchain.get_block_sums(&header.hash())?;

		// TODO - calculate fee for all txs
		let overage = self.fee() as i64;
		let io_sum = self.sum_commitments(overage, Some(block_sums.output_sum))?;

		// TODO - calculate sum of all offsets
		let offset = {
			let secp = static_secp_instance();
			let secp = secp.lock().unwrap();
			let key = self.offset.secret_key(&secp)?;
			secp.commit(0, key)?
		};
		let (kernel_sum, _) = self.sum_kernel_excesses(offset, Some(block_sums.kernel_sum));

		// TODO now do it all again with the new tx
		// TODO - consider adding the tx (temporarily) to the pool, then checking
		// validity???

		// and if everything is valid we can add the tx to the pool here

		self.transactions.insert(tx.hash(), Box::new(tx));

		panic!("this is as far as it goes...");
	}

	/// Whether the transaction is acceptable to the pool, given both how
	/// full the pool is and the transaction weight.
	fn is_acceptable(&self, tx: &Transaction) -> Result<(), PoolError> {
		if self.total_size() > self.config.max_pool_size {
			// TODO evict old/large transactions instead
			return Err(PoolError::OverCapacity);
		}

		// for a basic transaction (1 input, 2 outputs) -
		// (-1 * 1) + (4 * 2) + 1 = 8
		// 8 * 10 = 80
		if self.config.accept_fee_base > 0 {
			let threshold = (tx.tx_weight() as u64) * self.config.accept_fee_base;
			if tx.fee() < threshold {
				return Err(PoolError::LowFeeTransaction(threshold));
			}
		}
		Ok(())
	}

	/// Get the total size of the pool.
	pub fn total_size(&self) -> usize {
		self.transactions.len()
	}
}
