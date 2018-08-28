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

use chrono::prelude::{DateTime, Utc};

use core::consensus;
use core::core::hash::Hash;
use core::core::transaction::{self, Transaction};
use core::core::BlockHeader;

/// Dandelion relay timer
const DANDELION_RELAY_SECS: u64 = 600;

/// Dandelion embargo timer
const DANDELION_EMBARGO_SECS: u64 = 180;

/// Dandelion patience timer
const DANDELION_PATIENCE_SECS: u64 = 10;

/// Dandelion stem probability (stem 90% of the time, fluff 10%).
const DANDELION_STEM_PROBABILITY: usize = 90;

/// Configuration for "Dandelion".
/// Note: shared between p2p and pool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DandelionConfig {
	/// Choose new Dandelion relay peer every n secs.
	#[serde = "default_dandelion_relay_secs"]
	pub relay_secs: Option<u64>,
	/// Dandelion embargo, fluff and broadcast tx if not seen on network before
	/// embargo expires.
	#[serde = "default_dandelion_embargo_secs"]
	pub embargo_secs: Option<u64>,
	/// Dandelion patience timer, fluff/stem processing runs every n secs.
	/// Tx aggregation happens on stem txs received within this window.
	#[serde = "default_dandelion_patience_secs"]
	pub patience_secs: Option<u64>,
	/// Dandelion stem probability (stem 90% of the time, fluff 10% etc.)
	#[serde = "default_dandelion_stem_probability"]
	pub stem_probability: Option<usize>,
}

impl Default for DandelionConfig {
	fn default() -> DandelionConfig {
		DandelionConfig {
			relay_secs: default_dandelion_relay_secs(),
			embargo_secs: default_dandelion_embargo_secs(),
			patience_secs: default_dandelion_patience_secs(),
			stem_probability: default_dandelion_stem_probability(),
		}
	}
}

fn default_dandelion_relay_secs() -> Option<u64> {
	Some(DANDELION_RELAY_SECS)
}

fn default_dandelion_embargo_secs() -> Option<u64> {
	Some(DANDELION_EMBARGO_SECS)
}

fn default_dandelion_patience_secs() -> Option<u64> {
	Some(DANDELION_PATIENCE_SECS)
}

fn default_dandelion_stem_probability() -> Option<usize> {
	Some(DANDELION_STEM_PROBABILITY)
}

/// Transaction pool configuration
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PoolConfig {
	/// Base fee for a transaction to be accepted by the pool. The transaction
	/// weight is computed from its number of inputs, outputs and kernels and
	/// multiplied by the base fee to compare to the actual transaction fee.
	#[serde = "default_accept_fee_base"]
	pub accept_fee_base: u64,

	/// Maximum capacity of the pool in number of transactions
	#[serde = "default_max_pool_size"]
	pub max_pool_size: usize,
}

impl Default for PoolConfig {
	fn default() -> PoolConfig {
		PoolConfig {
			accept_fee_base: default_accept_fee_base(),
			max_pool_size: default_max_pool_size(),
		}
	}
}

fn default_accept_fee_base() -> u64 {
	consensus::MILLI_GRIN
}
fn default_max_pool_size() -> usize {
	50_000
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
	pub tx_at: DateTime<Utc>,
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
	/// Attempt to add a duplicate output to the pool.
	DuplicateCommitment,
	/// Attempt to add a duplicate tx to the pool.
	DuplicateTx,
	/// Other kinds of error (not yet pulled out into meaningful errors).
	Other(String),
}

impl From<transaction::Error> for PoolError {
	fn from(e: transaction::Error) -> PoolError {
		PoolError::InvalidTx(e)
	}
}

/// Interface that the pool requires from a blockchain implementation.
pub trait BlockChain: Sync + Send {
	/// Validate a vec of txs against known chain state at specific block
	/// after applying the pre_tx to the chain state.
	fn validate_raw_txs(
		&self,
		txs: Vec<transaction::Transaction>,
		pre_tx: Option<transaction::Transaction>,
		block_hash: &Hash,
	) -> Result<Vec<transaction::Transaction>, PoolError>;

	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_coinbase_maturity(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;

	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_tx_lock_height(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;

	fn chain_head(&self) -> Result<BlockHeader, PoolError>;
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
