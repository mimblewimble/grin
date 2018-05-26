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

//! The primary module containing the implementations of the transaction pool
//! and its top-level members.

use std::{error, fmt};
use time::Timespec;

use core::consensus;
use core::core::transaction;
use core::core::transaction::Transaction;

/// Transaction pool configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolConfig {
	/// Base fee for a transaction to be accepted by the pool. The transaction
	/// weight is computed from its number of inputs, outputs and kernels and
	/// multipled by the base fee to compare to the actual transaction fee.
	#[serde = "default_accept_fee_base"]
	pub accept_fee_base: u64,

	/// Maximum capacity of the pool in number of transactions
	#[serde = "default_max_pool_size"]
	pub max_pool_size: usize,

	/// Maximum capacity of the pool in number of transactions
	#[serde = "default_dandelion_probability"]
	pub dandelion_probability: usize,

	/// Default embargo for Dandelion transaction
	#[serde = "default_dandelion_embargo"]
	pub dandelion_embargo: i64,
}

impl Default for PoolConfig {
	fn default() -> PoolConfig {
		PoolConfig {
			accept_fee_base: default_accept_fee_base(),
			max_pool_size: default_max_pool_size(),
			dandelion_probability: default_dandelion_probability(),
			dandelion_embargo: default_dandelion_embargo(),
		}
	}
}

fn default_accept_fee_base() -> u64 {
	consensus::MILLI_GRIN
}
fn default_max_pool_size() -> usize {
	50_000
}
fn default_dandelion_probability() -> usize {
	90
}
fn default_dandelion_embargo() -> i64 {
	30
}

/// Represents a single entry in the pool.
/// A single (possibly aggregated) transaction.
#[derive(Clone, Debug)]
pub struct PoolEntry {
	/// The state of the pool entry.
	pub state: PoolEntryState,
	/// Info on where this tx originated from.
	pub src: TxSource,
	/// Timestamp of when this tx was originally added to the pool.
	pub tx_at: Timespec,
	/// The transaction itself.
	pub tx: Transaction,
}

/// The possible states a pool entry can be in.
#[derive(Clone, Debug, PartialEq)]
pub enum PoolEntryState {
	/// A new entry, not yet processed.
	Fresh,
	/// Tx to be included in the next "stem" run.
	ToStem,
	/// Tx previously "stemmed" and propagated.
	Stemmed,
	/// Tx to be included in the next "fluff" run.
	ToFluff,
	/// Tx previously "fluffed" and broadcast.
	Fluffed,
}

/// Placeholder: the data representing where we heard about a tx from.
///
/// Used to make decisions based on transaction acceptance priority from
/// various sources. For example, a node may want to bypass pool size
/// restrictions when accepting a transaction from a local wallet.
///
/// Most likely this will evolve to contain some sort of network identifier,
/// once we get a better sense of what transaction building might look like.
#[derive(Clone, Debug)]
pub struct TxSource {
	/// Human-readable name used for logging and errors.
	pub debug_name: String,
	/// Unique identifier used to distinguish this peer from others.
	pub identifier: String,
}

/// Possible errors when interacting with the transaction pool.
#[derive(Debug)]
pub enum PoolError {
	/// An invalid pool entry caused by underlying tx validation error
	InvalidTx(transaction::Error),
	/// Attempt to add a transaction to the pool with lock_height
	/// greater than height of current block
	ImmatureTransaction,
	/// Attempt to spend a coinbase output before it has sufficiently matured.
	ImmatureCoinbase,
	/// Problem propagating a stem tx to the next Dandelion relay node.
	DandelionError,
	/// Transaction pool is over capacity, can't accept more transactions
	OverCapacity,
	/// Transaction fee is too low given its weight
	LowFeeTransaction(u64),
	/// Other kinds of error (not yet pulled out into meaningful errors).
	Other(String),
}

impl From<transaction::Error> for PoolError {
	fn from(e: transaction::Error) -> PoolError {
		PoolError::InvalidTx(e)
	}
}

impl error::Error for PoolError {
	fn description(&self) -> &str {
		match *self {
			_ => "some kind of pool error",
		}
	}
}

impl fmt::Display for PoolError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			_ => write!(f, "some kind of pool error"),
		}
	}
}

/// Interface that the pool requires from a blockchain implementation.
pub trait BlockChain {
	/// Validate a vec of txs against the current chain state after applying the
	/// pre_tx to the chain state.
	fn validate_raw_txs(
		&self,
		txs: Vec<transaction::Transaction>,
		pre_tx: Option<transaction::Transaction>,
	) -> Result<Vec<transaction::Transaction>, PoolError>;

	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_coinbase_maturity(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;

	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_tx_lock_height(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;
}

/// Bridge between the transaction pool and the rest of the system. Handles
/// downstream processing of valid transactions by the rest of the system, most
/// importantly the broadcasting of transactions to our peers.
pub trait PoolAdapter: Send + Sync {
	/// The transaction pool has accepted this transactions as valid and added
	/// it to its internal cache.
	fn tx_accepted(&self, tx: &transaction::Transaction);
	/// The stem transaction pool has accepted this transactions as valid and
	/// added it to its internal cache, we have waited for the "patience" timer
	/// to fire and we now want to propagate the tx to the next Dandelion relay.
	fn stem_tx_accepted(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;
}

/// Dummy adapter used as a placeholder for real implementations
#[allow(dead_code)]
pub struct NoopAdapter {}

impl PoolAdapter for NoopAdapter {
	fn tx_accepted(&self, _: &transaction::Transaction) {}

	fn stem_tx_accepted(&self, _: &transaction::Transaction) -> Result<(), PoolError> {
		Ok(())
	}
}
