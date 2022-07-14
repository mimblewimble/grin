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

//! Owner API External Definition

use crate::chain::{Chain, SyncState};
use crate::core::core::hash::Hash;
use crate::handlers::chain_api::{ChainCompactHandler, ChainResetHandler, ChainValidationHandler};
use crate::handlers::peers_api::{PeerHandler, PeersConnectedHandler};
use crate::handlers::server_api::StatusHandler;
use crate::p2p::types::PeerInfoDisplay;
use crate::p2p::{self, PeerData};
use crate::rest::*;
use crate::types::Status;
use std::net::SocketAddr;
use std::sync::Weak;

/// Main interface into all node API functions.
/// Node APIs are split into two seperate blocks of functionality
/// called the ['Owner'](struct.Owner.html) and ['Foreign'](struct.Foreign.html) APIs
///
/// Methods in this API are intended to be 'single use'.
///

pub struct Owner {
	pub chain: Weak<Chain>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl Owner {
	/// Create a new API instance with the chain, transaction pool, peers and `sync_state`. All subsequent
	/// API calls will operate on this instance of node API.
	///
	/// # Arguments
	/// * `chain` - A non-owning reference of the chain.
	/// * `tx_pool` - A non-owning reference of the transaction pool.
	/// * `peers` - A non-owning reference of the peers.
	/// * `sync_state` - A non-owning reference of the `sync_state`.
	///
	/// # Returns
	/// * An instance of the Node holding references to the current chain, transaction pool, peers and sync_state.
	///

	pub fn new(chain: Weak<Chain>, peers: Weak<p2p::Peers>, sync_state: Weak<SyncState>) -> Self {
		Owner {
			chain,
			peers,
			sync_state,
		}
	}

	/// Returns various information about the node, the network and the current sync status.
	///
	/// # Returns
	/// * Result Containing:
	/// * A [`Status`](types/struct.Status.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_status(&self) -> Result<Status, Error> {
		let status_handler = StatusHandler {
			chain: self.chain.clone(),
			peers: self.peers.clone(),
			sync_state: self.sync_state.clone(),
		};
		status_handler.get_status()
	}

	/// Trigger a validation of the chain state.
	///
	/// # Arguments
	/// * `assume_valid_rangeproofs_kernels` -  if false, will validate rangeproofs, kernel signatures and sum of kernel excesses. if true, will only validate the sum of kernel excesses should equal the sum of unspent outputs minus total supply.
	///
	/// # Returns
	/// * Result Containing:
	/// * `Ok(())` if the validation was done successfully
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn validate_chain(&self, assume_valid_rangeproofs_kernels: bool) -> Result<(), Error> {
		let chain_validation_handler = ChainValidationHandler {
			chain: self.chain.clone(),
		};
		chain_validation_handler.validate_chain(assume_valid_rangeproofs_kernels)
	}

	/// Trigger a compaction of the chain state to regain storage space.
	///
	/// # Returns
	/// * Result Containing:
	/// * `Ok(())` if the compaction was done successfully
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn compact_chain(&self) -> Result<(), Error> {
		let chain_compact_handler = ChainCompactHandler {
			chain: self.chain.clone(),
		};
		chain_compact_handler.compact_chain()
	}

	pub fn reset_chain_head(&self, hash: String) -> Result<(), Error> {
		let hash =
			Hash::from_hex(&hash).map_err(|_| Error::RequestError("invalid header hash".into()))?;
		let handler = ChainResetHandler {
			chain: self.chain.clone(),
			sync_state: self.sync_state.clone(),
		};
		handler.reset_chain_head(hash)
	}

	pub fn invalidate_header(&self, hash: String) -> Result<(), Error> {
		let hash =
			Hash::from_hex(&hash).map_err(|_| Error::RequestError("invalid header hash".into()))?;
		let handler = ChainResetHandler {
			chain: self.chain.clone(),
			sync_state: self.sync_state.clone(),
		};
		handler.invalidate_header(hash)
	}

	/// Retrieves information about stored peers.
	/// If `None` is provided, will list all stored peers.
	///
	/// # Arguments
	/// * `addr` - the ip:port of the peer to get.
	///
	/// # Returns
	/// * Result Containing:
	/// * A vector of [`PeerData`](types/struct.PeerData.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_peers(&self, addr: Option<SocketAddr>) -> Result<Vec<PeerData>, Error> {
		let peer_handler = PeerHandler {
			peers: self.peers.clone(),
		};
		peer_handler.get_peers(addr)
	}

	/// Retrieves a list of all connected peers.
	///
	/// # Returns
	/// * Result Containing:
	/// * A vector of [`PeerInfoDisplay`](types/struct.PeerInfoDisplay.html)
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn get_connected_peers(&self) -> Result<Vec<PeerInfoDisplay>, Error> {
		let peers_connected_handler = PeersConnectedHandler {
			peers: self.peers.clone(),
		};
		peers_connected_handler.get_connected_peers()
	}

	/// Bans a specific peer.
	///
	/// # Arguments
	/// * `addr` - the ip:port of the peer to ban.
	///
	/// # Returns
	/// * Result Containing:
	/// * `Ok(())` if the path was correctly set
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn ban_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let peer_handler = PeerHandler {
			peers: self.peers.clone(),
		};
		peer_handler.ban_peer(addr)
	}

	/// Unbans a specific peer.
	///
	/// # Arguments
	/// * `addr` -  the ip:port of the peer to unban.
	///
	/// # Returns
	/// * Result Containing:
	/// * `Ok(())` if the unban was done successfully
	/// * or [`Error`](struct.Error.html) if an error is encountered.
	///

	pub fn unban_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let peer_handler = PeerHandler {
			peers: self.peers.clone(),
		};
		peer_handler.unban_peer(addr)
	}
}
