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
use crate::handlers::server_api::StatusHandler;
use crate::p2p;
use crate::pool;
use crate::rest::*;
use crate::types::Status;
use crate::util::RwLock;
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

	/// UNFINISHED
	/// Returns various information about the node, the network and the current sync status.
	///
	/// # Returns
	/// * a result containing:
	/// * The current status [Status](../grin/slate/struct.Slate.html),
	/// * The first `bool` element indicates whether the data was successfully
	/// refreshed from the node (note this may be false even if the `refresh_from_node`
	/// argument was set to `true`.
	/// * or [`libwallet::Error`](../grin_wallet_libwallet/struct.Error.html) if an error is encountered.
	///
	/// # Example
	/// Set up as in [`new`](struct.Owner.html#method.new) method above.
	/// ```
	/// # grin_wallet_api::doctest_helper_setup_doc_env!(wallet, wallet_config);
	///
	/// let mut api_owner = Owner::new(wallet.clone());
	/// let update_from_node = true;
	/// let minimum_confirmations=10;
	///
	/// // Return summary info for active account
	/// let result = api_owner.retrieve_summary_info(None, update_from_node, minimum_confirmations);
	///
	/// if let Ok((was_updated, summary_info)) = result {
	///		//...
	/// }
	/// ```
	pub fn get_status(&self) -> Result<Status, Error> {
		let status_handler = StatusHandler {
			chain: self.chain.clone(),
			peers: self.peers.clone(),
			sync_state: self.sync_state.clone(),
		};
		status_handler.get_status()
	}
}
