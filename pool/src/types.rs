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

use self::core::core::block;
use self::core::core::committed;
use self::core::core::hash::Hash;
use self::core::core::transaction::{self, Transaction};
use self::core::core::{BlockHeader, BlockSums};
use self::core::{consensus, global};
use failure::Fail;
use grin_core as core;
use grin_keychain as keychain;

/// Dandelion "epoch" length.
const DANDELION_EPOCH_SECS: u16 = 600;

/// Dandelion embargo timer.
const DANDELION_EMBARGO_SECS: u16 = 180;

/// Dandelion aggregation timer.
const DANDELION_AGGREGATION_SECS: u16 = 30;

/// Dandelion stem probability (stem 90% of the time, fluff 10%).
const DANDELION_STEM_PROBABILITY: u8 = 90;

/// Configuration for "Dandelion".
/// Note: shared between p2p and pool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DandelionConfig {
	/// Length of each "epoch".
	#[serde(default = "default_dandelion_epoch_secs")]
	pub epoch_secs: Option<u16>,
	/// Dandelion embargo timer. Fluff and broadcast individual txs if not seen
	/// on network before embargo expires.
	#[serde(default = "default_dandelion_embargo_secs")]
	pub embargo_secs: Option<u16>,
	/// Dandelion aggregation timer.
	#[serde(default = "default_dandelion_aggregation_secs")]
	pub aggregation_secs: Option<u16>,
	/// Dandelion stem probability (stem 90% of the time, fluff 10% etc.)
	#[serde(default = "default_dandelion_stem_probability")]
	pub stem_probability: Option<u8>,
}

impl Default for DandelionConfig {
	fn default() -> DandelionConfig {
		DandelionConfig {
			epoch_secs: default_dandelion_epoch_secs(),
			embargo_secs: default_dandelion_embargo_secs(),
			aggregation_secs: default_dandelion_aggregation_secs(),
			stem_probability: default_dandelion_stem_probability(),
		}
	}
}

fn default_dandelion_epoch_secs() -> Option<u16> {
	Some(DANDELION_EPOCH_SECS)
}

fn default_dandelion_embargo_secs() -> Option<u16> {
	Some(DANDELION_EMBARGO_SECS)
}

fn default_dandelion_aggregation_secs() -> Option<u16> {
	Some(DANDELION_AGGREGATION_SECS)
}

fn default_dandelion_stem_probability() -> Option<u8> {
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

	/// Maximum capacity of the pool in number of transactions
	#[serde = "default_max_stempool_size"]
	pub max_stempool_size: usize,

	/// Maximum total weight of transactions that can get selected to build a
	/// block from. Allows miners to restrict the maximum weight of their
	/// blocks.
	#[serde = "default_mineable_max_weight"]
	pub mineable_max_weight: usize,
}

impl Default for PoolConfig {
	fn default() -> PoolConfig {
		PoolConfig {
			accept_fee_base: default_accept_fee_base(),
			max_pool_size: default_max_pool_size(),
			max_stempool_size: default_max_stempool_size(),
			mineable_max_weight: default_mineable_max_weight(),
		}
	}
}

fn default_accept_fee_base() -> u64 {
	consensus::MILLI_GRIN
}
fn default_max_pool_size() -> usize {
	50_000
}
fn default_max_stempool_size() -> usize {
	50_000
}
fn default_mineable_max_weight() -> usize {
	global::max_block_weight()
}

/// Represents a single entry in the pool.
/// A single (possibly aggregated) transaction.
#[derive(Clone, Debug)]
pub struct PoolEntry {
	/// Info on where this tx originated from.
	pub src: TxSource,
	/// Timestamp of when this tx was originally added to the pool.
	pub tx_at: DateTime<Utc>,
	/// The transaction itself.
	pub tx: Transaction,
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
#[derive(Debug, Fail, PartialEq)]
pub enum PoolError {
	/// An invalid pool entry caused by underlying tx validation error
	#[fail(display = "Invalid Tx {}", _0)]
	InvalidTx(transaction::Error),
	/// An invalid pool entry caused by underlying block validation error
	#[fail(display = "Invalid Block {}", _0)]
	InvalidBlock(block::Error),
	/// Underlying keychain error.
	#[fail(display = "Keychain error {}", _0)]
	Keychain(keychain::Error),
	/// Underlying "committed" error.
	#[fail(display = "Committed error {}", _0)]
	Committed(committed::Error),
	/// Attempt to add a transaction to the pool with lock_height
	/// greater than height of current block
	#[fail(display = "Immature transaction")]
	ImmatureTransaction,
	/// Attempt to spend a coinbase output before it has sufficiently matured.
	#[fail(display = "Immature coinbase")]
	ImmatureCoinbase,
	/// Problem propagating a stem tx to the next Dandelion relay node.
	#[fail(display = "Dandelion error")]
	DandelionError,
	/// Transaction pool is over capacity, can't accept more transactions
	#[fail(display = "Over capacity")]
	OverCapacity,
	/// Transaction fee is too low given its weight
	#[fail(display = "Low fee transaction {}", _0)]
	LowFeeTransaction(u64),
	/// Attempt to add a duplicate output to the pool.
	#[fail(display = "Duplicate commitment")]
	DuplicateCommitment,
	/// Attempt to add a duplicate tx to the pool.
	#[fail(display = "Duplicate tx")]
	DuplicateTx,
	/// Other kinds of error (not yet pulled out into meaningful errors).
	#[fail(display = "General pool error {}", _0)]
	Other(String),
}

impl From<transaction::Error> for PoolError {
	fn from(e: transaction::Error) -> PoolError {
		PoolError::InvalidTx(e)
	}
}

impl From<block::Error> for PoolError {
	fn from(e: block::Error) -> PoolError {
		PoolError::InvalidBlock(e)
	}
}

impl From<keychain::Error> for PoolError {
	fn from(e: keychain::Error) -> PoolError {
		PoolError::Keychain(e)
	}
}

impl From<committed::Error> for PoolError {
	fn from(e: committed::Error) -> PoolError {
		PoolError::Committed(e)
	}
}

/// Interface that the pool requires from a blockchain implementation.
pub trait BlockChain: Sync + Send {
	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_coinbase_maturity(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;

	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_tx_lock_height(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;

	fn validate_tx(&self, tx: &Transaction) -> Result<(), PoolError>;

	fn chain_head(&self) -> Result<BlockHeader, PoolError>;

	fn get_block_header(&self, hash: &Hash) -> Result<BlockHeader, PoolError>;
	fn get_block_sums(&self, hash: &Hash) -> Result<BlockSums, PoolError>;
}

/// Bridge between the transaction pool and the rest of the system. Handles
/// downstream processing of valid transactions by the rest of the system, most
/// importantly the broadcasting of transactions to our peers.
pub trait PoolAdapter: Send + Sync {
	/// The transaction pool has accepted this transaction as valid.
	fn tx_accepted(&self, tx: &transaction::Transaction);

	/// The stem transaction pool has accepted this transactions as valid.
	fn stem_tx_accepted(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;
}

/// Dummy adapter used as a placeholder for real implementations
#[allow(dead_code)]
pub struct NoopAdapter {}

impl PoolAdapter for NoopAdapter {
	fn tx_accepted(&self, _tx: &transaction::Transaction) {}
	fn stem_tx_accepted(&self, _tx: &transaction::Transaction) -> Result<(), PoolError> {
		Ok(())
	}
}
