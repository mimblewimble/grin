// Copyright 2019 The Grin Developers
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

//! Node API External Definition

use crate::chain::{Chain, SyncState};
use crate::core::core::hash::Hash;
use crate::core::core::transaction::Transaction;
use crate::handlers::blocks_api::{BlockHandler, HeaderHandler};
use crate::handlers::chain_api::{
	ChainCompactHandler, ChainHandler, ChainValidationHandler, KernelHandler, OutputHandler,
};
use crate::handlers::peers_api::{PeerHandler, PeersConnectedHandler};
use crate::handlers::pool_api::PoolHandler;
use crate::handlers::server_api::StatusHandler;
use crate::handlers::version_api::VersionHandler;
use crate::p2p::types::PeerInfoDisplay;
use crate::p2p::{self, PeerData};
use crate::pool::{self, PoolEntry};
use crate::rest::*;
use crate::types::{
	BlockHeaderPrintable, BlockPrintable, LocatedTxKernel, OutputListing, OutputPrintable, Status,
	Tip, Version,
};
use crate::util::RwLock;
use std::net::SocketAddr;
use std::sync::Weak;

/// Main interface into all node API functions.
/// Node API functionality are in the ['Node'](struct.Node.html)
///
/// Methods in this API are intended to be 'single use'.
///

pub struct Node {
	pub chain: Weak<Chain>,
	pub tx_pool: Weak<RwLock<pool::TransactionPool>>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl Node {
	pub fn new(
		chain: Weak<Chain>,
		tx_pool: Weak<RwLock<pool::TransactionPool>>,
		peers: Weak<p2p::Peers>,
		sync_state: Weak<SyncState>,
	) -> Self {
		Node {
			chain,
			tx_pool,
			peers,
			sync_state,
		}
	}

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
		block_handler.get_block(&hash, true, true)
	}

	pub fn get_status(&self) -> Result<Status, Error> {
		let status_handler = StatusHandler {
			chain: self.chain.clone(),
			peers: self.peers.clone(),
			sync_state: self.sync_state.clone(),
		};
		status_handler.get_status()
	}

	pub fn get_version(&self) -> Result<Version, Error> {
		let version_handler = VersionHandler {
			chain: self.chain.clone(),
		};
		version_handler.get_version()
	}

	pub fn get_tip(&self) -> Result<Tip, Error> {
		let chain_handler = ChainHandler {
			chain: self.chain.clone(),
		};
		chain_handler.get_tip()
	}

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
		output_handler.get_outputs(
			commits,
			start_height,
			end_height,
			include_proof,
			include_merkle_proof,
		)
	}

	pub fn get_unspent_outputs(
		&self,
		start_index: u64,
		max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, Error> {
		let output_handler = OutputHandler {
			chain: self.chain.clone(),
		};
		output_handler.get_unspent_outputs(start_index, max, include_proof)
	}

	pub fn validate_chain(&self) -> Result<(), Error> {
		let chain_validation_handler = ChainValidationHandler {
			chain: self.chain.clone(),
		};
		chain_validation_handler.validate_chain()
	}

	pub fn compact_chain(&self) -> Result<(), Error> {
		let chain_compact_handler = ChainCompactHandler {
			chain: self.chain.clone(),
		};
		chain_compact_handler.compact_chain()
	}

	pub fn get_peers(&self, addr: Option<SocketAddr>) -> Result<Vec<PeerData>, Error> {
		let peer_handler = PeerHandler {
			peers: self.peers.clone(),
		};
		peer_handler.get_peers(addr)
	}

	pub fn get_connected_peers(&self) -> Result<Vec<PeerInfoDisplay>, Error> {
		let peers_connected_handler = PeersConnectedHandler {
			peers: self.peers.clone(),
		};
		peers_connected_handler.get_connected_peers()
	}

	pub fn ban_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let peer_handler = PeerHandler {
			peers: self.peers.clone(),
		};
		peer_handler.ban_peer(addr)
	}

	pub fn unban_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let peer_handler = PeerHandler {
			peers: self.peers.clone(),
		};
		peer_handler.unban_peer(addr)
	}

	pub fn get_pool_size(&self) -> Result<usize, Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.get_pool_size()
	}

	pub fn get_stempool_size(&self) -> Result<usize, Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.get_stempool_size()
	}
	pub fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.get_unconfirmed_transactions()
	}
	pub fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), Error> {
		let pool_handler = PoolHandler {
			tx_pool: self.tx_pool.clone(),
		};
		pool_handler.push_transaction(tx, fluff)
	}
}
