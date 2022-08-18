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

//! Foreign API External Definition

use crate::chain::{Chain, SyncState};
use crate::core::core::hash::Hash;
use crate::core::core::transaction::Transaction;
use crate::handlers::blocks_api::{BlockHandler, HeaderHandler};
use crate::handlers::chain_api::{ChainHandler, KernelHandler, OutputHandler};
use crate::handlers::pool_api::PoolHandler;
use crate::handlers::transactions_api::TxHashSetHandler;
use crate::handlers::version_api::VersionHandler;
use crate::pool::{self, BlockChain, PoolAdapter, PoolEntry};
use crate::rest::*;
use crate::types::{
	BlockHeaderPrintable, BlockPrintable, LocatedTxKernel, OutputListing, OutputPrintable, Tip,
	Version,
};
use crate::util::RwLock;
use std::sync::Weak;

/// Main interface into all node API functions.
/// Node APIs are split into two seperate blocks of functionality
/// called the ['Owner'](struct.Owner.html) and ['Foreign'](struct.Foreign.html) APIs
///
/// Methods in this API are intended to be 'single use'.
///

pub struct Foreign<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	pub chain: Weak<Chain>,
	pub tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
	pub sync_state: Weak<SyncState>,
}

impl<B, P> Foreign<B, P>
where
	B: BlockChain,
	P: PoolAdapter,
{
	/// Create a new API instance with the chain, transaction pool, peers and `sync_state`. All subsequent
	/// API calls will operate on this instance of node API.
	///
	/// # Arguments
	/// * `chain` - A non-owning reference of the chain.
	/// * `tx_pool` - A non-owning reference of the transaction pool.
	/// * `sync_state` - A non-owning reference of the `sync_state`.
	///
	/// # Returns
	/// * An instance of the Node holding references to the current chain, transaction pool, peers and sync_state.
	///

	pub fn new(
		chain: Weak<Chain>,
		tx_pool: Weak<RwLock<pool::TransactionPool<B, P>>>,
		sync_state: Weak<SyncState>,
	) -> Self {
		Foreign {
			chain,
			tx_pool,
			sync_state,
		}
	}

	/// Gets block header given either a height, a hash or an unspent output commitment. Only one parameters is needed.
	/// If multiple parameters are provided only the first one in the list is used.
	///
	/// # Arguments
	/// * `height` - block height.
	/// * `hash` - block hash.
	/// * `commit` - output commitment.
	///
	/// # Returns
	/// * Result Containing:
	/// * A [`BlockHeaderPrintable`](types/struct.BlockHeaderPrintable.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_header(
		&self,
		height: Option<u64>,
		hash: Option<Hash>,
		commit: Option<String>,
	) -> Result<BlockHeaderPrintable, Error> {
		let header_handler = HeaderHandler {
			chain: self.chain.clone(),
		};
		let hash = header_handler.parse_inputs(height, hash, commit)?;
		header_handler.get_header_v2(&hash)
	}

	/// Gets block details given either a height, a hash or an unspent output commitment. Only one parameters is needed.
	/// If multiple parameters are provided only the first one in the list is used.
	///
	/// # Arguments
	/// * `height` - block height.
	/// * `hash` - block hash.
	/// * `commit` - output commitment.
	///
	/// # Returns
	/// * Result Containing:
	/// * A [`BlockPrintable`](types/struct.BlockPrintable.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_block(
		&self,
		height: Option<u64>,
		hash: Option<Hash>,
		commit: Option<String>,
	) -> Result<BlockPrintable, Error> {
		let block_handler = BlockHandler {
			chain: self.chain.clone(),
		};
		let hash = block_handler.parse_inputs(height, hash, commit)?;

		// We include the rangeproof by default.
		let include_proof = true;

		// We do *not* include the Merkle proof.
		// These are not actively used and expensive to generate for historical blocks.
		let include_merkle_proof = false;

		block_handler.get_block(&hash, include_proof, include_merkle_proof)
	}

	/// Returns the node version and block header version (used by grin-wallet).
	///
	/// # Returns
	/// * Result Containing:
	/// * A [`Version`](types/struct.Version.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_version(&self) -> Result<Version, Error> {
		let version_handler = VersionHandler {
			chain: self.chain.clone(),
		};
		version_handler.get_version()
	}

	/// Returns details about the state of the current fork tip.
	///
	/// # Returns
	/// * Result Containing:
	/// * A [`Tip`](types/struct.Tip.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_tip(&self) -> Result<Tip, Error> {
		let chain_handler = ChainHandler {
			chain: self.chain.clone(),
		};
		chain_handler.get_tip()
	}

	/// Returns a [`LocatedTxKernel`](types/struct.LocatedTxKernel.html) based on the kernel excess.
	/// The `min_height` and `max_height` parameters are both optional.
	/// If not supplied, `min_height` will be set to 0 and `max_height` will be set to the head of the chain.
	/// The method will start at the block height `max_height` and traverse the kernel MMR backwards,
	/// until either the kernel is found or `min_height` is reached.
	///
	/// # Arguments
	/// * `excess` - kernel excess to look for.
	/// * `min_height` - minimum height to stop the lookup.
	/// * `max_height` - maximum height to start the lookup.
	///
	/// # Returns
	/// * Result Containing:
	/// * A [`LocatedTxKernel`](types/struct.LocatedTxKernel.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_kernel(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, Error> {
		let kernel_handler = KernelHandler {
			chain: self.chain.clone(),
		};
		kernel_handler.get_kernel_v2(excess, min_height, max_height)
	}

	/// Retrieves details about specifics outputs. Supports retrieval of multiple outputs in a single request.
	/// Support retrieval by both commitment string and block height.
	///
	/// # Arguments
	/// * `commits` - a vector of unspent output commitments.
	/// * `start_height` - start height to start the lookup.
	/// * `end_height` - end height to stop the lookup.
	/// * `include_proof` - whether or not to include the range proof in the response.
	/// * `include_merkle_proof` - whether or not to include the merkle proof in the response.
	///
	/// # Returns
	/// * Result Containing:
	/// * An [`OutputPrintable`](types/struct.OutputPrintable.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_outputs(
		&self,
		commits: Option<Vec<String>>,
		start_height: Option<u64>,
		end_height: Option<u64>,
		include_proof: Option<bool>,
		include_merkle_proof: Option<bool>,
	) -> Result<Vec<OutputPrintable>, Error> {
		let output_handler = OutputHandler {
			chain: self.chain.clone(),
		};
		output_handler.get_outputs_v2(
			commits,
			start_height,
			end_height,
			include_proof,
			include_merkle_proof,
		)
	}

	/// UTXO traversal. Retrieves last utxos since a `start_index` until a `max`.
	///
	/// # Arguments
	/// * `start_index` - start index in the MMR.
	/// * `end_index` - optional index so stop in the MMR.
	/// * `max` - max index in the MMR.
	/// * `include_proof` - whether or not to include the range proof in the response.
	///
	/// # Returns
	/// * Result Containing:
	/// * An [`OutputListing`](types/struct.OutputListing.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_unspent_outputs(
		&self,
		start_index: u64,
		end_index: Option<u64>,
		max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, Error> {
		let output_handler = OutputHandler {
			chain: self.chain.clone(),
		};
		output_handler.get_unspent_outputs(start_index, end_index, max, include_proof)
	}

	/// Retrieves the PMMR indices based on the provided block height(s).
	///
	/// # Arguments
	/// * `start_block_height` - start index in the MMR.
	/// * `end_block_height` - optional index so stop in the MMR.
	///
	/// # Returns
	/// * Result Containing:
	/// * An [`OutputListing`](types/struct.OutputListing.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_pmmr_indices(
		&self,
		start_block_height: u64,
		end_block_height: Option<u64>,
	) -> Result<OutputListing, Error> {
		let txhashset_handler = TxHashSetHandler {
			chain: self.chain.clone(),
		};
		txhashset_handler.block_height_range_to_pmmr_indices(start_block_height, end_block_height)
	}

	/// Returns the number of transaction in the transaction pool.
	///
	/// # Returns
	/// * Result Containing:
	/// * `usize`
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_pool_size(&self) -> Result<usize, Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.get_pool_size()
	}

	/// Returns the number of transaction in the stem transaction pool.
	///
	/// # Returns
	/// * Result Containing:
	/// * `usize`
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_stempool_size(&self) -> Result<usize, Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.get_stempool_size()
	}

	/// Returns the unconfirmed transactions in the transaction pool.
	/// Will not return transactions in the stempool.
	///
	/// # Returns
	/// * Result Containing:
	/// * A vector of [`PoolEntry`](types/struct.PoolEntry.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.get_unconfirmed_transactions()
	}

	/// Push new transaction to our local transaction pool.
	///
	/// # Arguments
	/// * `tx` - the Grin transaction to push.
	/// * `fluff` - boolean to bypass Dandelion relay.
	///
	/// # Returns
	/// * Result Containing:
	/// * `Ok(())` if the transaction was pushed successfully
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///
	pub fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.push_transaction(tx, fluff)
	}
}
