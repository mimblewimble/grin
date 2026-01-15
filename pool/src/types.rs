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

//! The primary module containing the implementations of the transaction pool
//! and its top-level members.

use self::core::consensus;
use self::core::core::block;
use self::core::core::committed;
use self::core::core::hash::Hash;
use self::core::core::transaction::{self, Transaction};
use self::core::core::{BlockHeader, BlockSums, Inputs, OutputIdentifier};
use self::core::global::DEFAULT_ACCEPT_FEE_BASE;
use chrono::prelude::*;
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

/// Always stem our (pushed via api) txs?
/// Defaults to true to match the Dandelion++ paper.
/// But can be overridden to allow a node to fluff our txs if desired.
/// If set to false we will stem/fluff our txs as per current epoch.
const DANDELION_ALWAYS_STEM_OUR_TXS: bool = true;

/// Configuration for "Dandelion".
/// Note: shared between p2p and pool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DandelionConfig {
	/// Length of each "epoch".
	#[serde(default = "default_dandelion_epoch_secs")]
	pub epoch_secs: u16,
	/// Dandelion embargo timer. Fluff and broadcast individual txs if not seen
	/// on network before embargo expires.
	#[serde(default = "default_dandelion_embargo_secs")]
	pub embargo_secs: u16,
	/// Dandelion aggregation timer.
	#[serde(default = "default_dandelion_aggregation_secs")]
	pub aggregation_secs: u16,
	/// Dandelion stem probability (stem 90% of the time, fluff 10% etc.)
	#[serde(default = "default_dandelion_stem_probability")]
	pub stem_probability: u8,
	/// Default to always stem our txs as described in Dandelion++ paper.
	#[serde(default = "default_dandelion_always_stem_our_txs")]
	pub always_stem_our_txs: bool,
}

impl Default for DandelionConfig {
	fn default() -> DandelionConfig {
		DandelionConfig {
			epoch_secs: default_dandelion_epoch_secs(),
			embargo_secs: default_dandelion_embargo_secs(),
			aggregation_secs: default_dandelion_aggregation_secs(),
			stem_probability: default_dandelion_stem_probability(),
			always_stem_our_txs: default_dandelion_always_stem_our_txs(),
		}
	}
}

fn default_dandelion_epoch_secs() -> u16 {
	DANDELION_EPOCH_SECS
}

fn default_dandelion_embargo_secs() -> u16 {
	DANDELION_EMBARGO_SECS
}

fn default_dandelion_aggregation_secs() -> u16 {
	DANDELION_AGGREGATION_SECS
}

fn default_dandelion_stem_probability() -> u8 {
	DANDELION_STEM_PROBABILITY
}

fn default_dandelion_always_stem_our_txs() -> bool {
	DANDELION_ALWAYS_STEM_OUR_TXS
}

/// Transaction pool configuration
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PoolConfig {
	/// Base fee for a transaction to be accepted by the pool. The transaction
	/// weight is computed from its number of inputs, outputs and kernels and
	/// multiplied by the base fee to compare to the actual transaction fee.
	#[serde(default = "default_accept_fee_base")]
	pub accept_fee_base: u64,

	// Reorg cache retention period in minute.
	// The reorg cache repopulates local mempool in a reorg scenario.
	#[serde(default = "default_reorg_cache_period")]
	pub reorg_cache_period: u32,

	/// Maximum capacity of the pool in number of transactions
	#[serde(default = "default_max_pool_size")]
	pub max_pool_size: usize,

	/// Maximum capacity of the pool in number of transactions
	#[serde(default = "default_max_stempool_size")]
	pub max_stempool_size: usize,

	/// Maximum total weight of transactions that can get selected to build a
	/// block from. Allows miners to restrict the maximum weight of their
	/// blocks.
	#[serde(default = "default_mineable_max_weight")]
	pub mineable_max_weight: u64,
}

impl Default for PoolConfig {
	fn default() -> PoolConfig {
		PoolConfig {
			accept_fee_base: default_accept_fee_base(),
			reorg_cache_period: default_reorg_cache_period(),
			max_pool_size: default_max_pool_size(),
			max_stempool_size: default_max_stempool_size(),
			mineable_max_weight: default_mineable_max_weight(),
		}
	}
}

/// make output (of weight 21) cost about 1 Grin-cent by default, keeping a round number
pub fn default_accept_fee_base() -> u64 {
	DEFAULT_ACCEPT_FEE_BASE
}
fn default_reorg_cache_period() -> u32 {
	30
}
fn default_max_pool_size() -> usize {
	50_000
}
fn default_max_stempool_size() -> usize {
	50_000
}
fn default_mineable_max_weight() -> u64 {
	consensus::MAX_BLOCK_WEIGHT
}

/// Represents a single entry in the pool.
/// A single (possibly aggregated) transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolEntry {
	/// Info on where this tx originated from.
	pub src: TxSource,
	/// Timestamp of when this tx was originally added to the pool.
	pub tx_at: DateTime<Utc>,
	/// The transaction itself.
	pub tx: Transaction,
}

impl PoolEntry {
	pub fn new(tx: Transaction, src: TxSource) -> PoolEntry {
		PoolEntry {
			src,
			tx_at: Utc::now(),
			tx,
		}
	}
}

/// Used to make decisions based on transaction acceptance priority from
/// various sources. For example, a node may want to bypass pool size
/// restrictions when accepting a transaction from a local wallet.
///
/// Most likely this will evolve to contain some sort of network identifier,
/// once we get a better sense of what transaction building might look like.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TxSource {
	PushApi,
	Broadcast,
	Fluff,
	EmbargoExpired,
	Deaggregate,
}

impl TxSource {
	/// Convenience fn for checking if this tx was sourced via the push api.
	pub fn is_pushed(&self) -> bool {
		match self {
			TxSource::PushApi => true,
			_ => false,
		}
	}
}

/// Possible errors when interacting with the transaction pool.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PoolError {
	/// An invalid pool entry caused by underlying tx validation error
	#[error("Invalid Tx {0}")]
	InvalidTx(transaction::Error),
	/// An invalid pool entry caused by underlying block validation error
	#[error("Invalid Block {0}")]
	InvalidBlock(block::Error),
	/// Underlying keychain error.
	#[error("Keychain error {0}")]
	Keychain(keychain::Error),
	/// Underlying "committed" error.
	#[error("Committed error {0}")]
	Committed(committed::Error),
	/// Attempt to add a transaction to the pool with lock_height
	/// greater than height of current block
	#[error("Immature transaction")]
	ImmatureTransaction,
	/// Attempt to spend a coinbase output before it has sufficiently matured.
	#[error("Immature coinbase")]
	ImmatureCoinbase,
	/// Problem propagating a stem tx to the next Dandelion relay node.
	#[error("Dandelion error")]
	DandelionError,
	/// Transaction pool is over capacity, can't accept more transactions
	#[error("Over capacity")]
	OverCapacity,
	/// Transaction fee is too low given its weight
	#[error("Low fee transaction {0}")]
	LowFeeTransaction(u64),
	/// Attempt to add a duplicate output to the pool.
	#[error("Duplicate commitment")]
	DuplicateCommitment,
	/// Attempt to add a duplicate tx to the pool.
	#[error("Duplicate tx")]
	DuplicateTx,
	/// NRD kernels will not be accepted by the txpool/stempool pre-HF3.
	#[error("NRD kernel pre-HF3")]
	NRDKernelPreHF3,
	/// NRD kernels are not valid if disabled locally via "feature flag".
	#[error("NRD kernel not enabled")]
	NRDKernelNotEnabled,
	/// NRD kernels are not valid if relative_height rule not met.
	#[error("NRD kernel relative height")]
	NRDKernelRelativeHeight,
	/// Other kinds of error (not yet pulled out into meaningful errors).
	#[error("General pool error {0}")]
	Other(String),
}

impl From<transaction::Error> for PoolError {
	fn from(e: transaction::Error) -> PoolError {
		match e {
			transaction::Error::InvalidNRDRelativeHeight => PoolError::NRDKernelRelativeHeight,
			e @ _ => PoolError::InvalidTx(e),
		}
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
	fn verify_coinbase_maturity(&self, inputs: &Inputs) -> Result<(), PoolError>;

	/// Verify any coinbase outputs being spent
	/// have matured sufficiently.
	fn verify_tx_lock_height(&self, tx: &transaction::Transaction) -> Result<(), PoolError>;

	/// Validate a transaction against the current utxo.
	fn validate_tx(&self, tx: &Transaction) -> Result<(), PoolError>;

	/// Validate inputs against the current utxo.
	/// Returns the vec of output identifiers that would be spent
	/// by these inputs if they can all be successfully spent.
	fn validate_inputs(&self, inputs: &Inputs) -> Result<Vec<OutputIdentifier>, PoolError>;

	fn chain_head(&self) -> Result<BlockHeader, PoolError>;

	fn get_block_header(&self, hash: &Hash) -> Result<BlockHeader, PoolError>;
	fn get_block_sums(&self, hash: &Hash) -> Result<BlockSums, PoolError>;
}

/// Bridge between the transaction pool and the rest of the system. Handles
/// downstream processing of valid transactions by the rest of the system, most
/// importantly the broadcasting of transactions to our peers.
pub trait PoolAdapter: Send + Sync {
	/// The transaction pool has accepted this transaction as valid.
	fn tx_accepted(&self, entry: &PoolEntry);

	/// The stem transaction pool has accepted this transactions as valid.
	fn stem_tx_accepted(&self, entry: &PoolEntry) -> Result<(), PoolError>;
}

/// Dummy adapter used as a placeholder for real implementations
#[allow(dead_code)]
pub struct NoopPoolAdapter {}

impl PoolAdapter for NoopPoolAdapter {
	fn tx_accepted(&self, _entry: &PoolEntry) {}
	fn stem_tx_accepted(&self, _entry: &PoolEntry) -> Result<(), PoolError> {
		Ok(())
	}
}
