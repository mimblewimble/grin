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

use chrono::prelude::{DateTime, Utc};
use chrono::Duration;
use std::sync::Arc;

use chain;
use common::types::{Error, SyncState, SyncStatus};
use core::core::hash::Hashed;
use core::global;
use p2p::{self, Peer};
use util::LOGGER;

/// Fast sync has 3 "states":
/// * syncing headers
/// * once all headers are sync'd, requesting the txhashset state
/// * once we have the state, get blocks after that
///
/// The StateSync struct implements and monitors the middle step.
pub struct StateSync {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	archive_mode: bool,

	prev_fast_sync: Option<DateTime<Utc>>,
	fast_sync_peer: Option<Arc<Peer>>,
}

impl StateSync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
		archive_mode: bool,
	) -> StateSync {
		StateSync {
			sync_state,
			peers,
			chain,
			archive_mode,
			prev_fast_sync: None,
			fast_sync_peer: None,
		}
	}

	/// Check whether state sync should run and triggers a state download when
	/// it's time (we have all headers). Returns true as long as state sync
	/// needs monitoring, false when it's either done or turned off.
	pub fn check_run(
		&mut self,
		header_head: &chain::Tip,
		head: &chain::Tip,
		highest_height: u64,
	) -> bool {
		let need_state_sync = !self.archive_mode
			&& highest_height.saturating_sub(head.height) > global::cut_through_horizon() as u64;
		if !need_state_sync {
			return false;
		}

		let mut sync_need_restart = false;

		// check sync error
		{
			let clone = self.sync_state.sync_error();
			if let Some(ref sync_error) = *clone.read().unwrap() {
				error!(
					LOGGER,
					"fast_sync: error = {:?}. restart fast sync", sync_error
				);
				sync_need_restart = true;
			}
			drop(clone);
		}

		// check peer connection status of this sync
		if let Some(ref peer) = self.fast_sync_peer {
			if !peer.is_connected() && SyncStatus::TxHashsetDownload == self.sync_state.status() {
				sync_need_restart = true;
				info!(
					LOGGER,
					"fast_sync: peer connection lost: {:?}. restart", peer.info.addr,
				);
			}
		}

		if sync_need_restart {
			self.fast_sync_reset();
			self.sync_state.clear_sync_error();
		}

		// run fast sync if applicable, normally only run one-time, except restart in error
		if header_head.height == highest_height {
			let (go, download_timeout) = self.fast_sync_due();

			if download_timeout && SyncStatus::TxHashsetDownload == self.sync_state.status() {
				error!(
					LOGGER,
					"fast_sync: TxHashsetDownload status timeout in 10 minutes!"
				);
				self.sync_state
					.set_sync_error(Error::P2P(p2p::Error::Timeout));
			}

			if go {
				self.fast_sync_peer = None;
				match self.request_state(&header_head) {
					Ok(peer) => {
						self.fast_sync_peer = Some(peer);
					}
					Err(e) => self.sync_state.set_sync_error(Error::P2P(e)),
				}
				self.sync_state.update(SyncStatus::TxHashsetDownload);
			}
		}
		true
	}

	fn request_state(&self, header_head: &chain::Tip) -> Result<Arc<Peer>, p2p::Error> {
		let horizon = global::cut_through_horizon() as u64;

		if let Some(peer) = self.peers.most_work_peer() {
			// ask for txhashset at 90% of horizon, this still leaves time for download
			// and validation to happen and stay within horizon
			let mut txhashset_head = self
				.chain
				.get_block_header(&header_head.prev_block_h)
				.unwrap();
			for _ in 0..(horizon - horizon / 10) {
				txhashset_head = self
					.chain
					.get_block_header(&txhashset_head.previous)
					.unwrap();
			}
			let bhash = txhashset_head.hash();
			debug!(
				LOGGER,
				"fast_sync: before txhashset request, header head: {} / {}, txhashset_head: {} / {}",
				header_head.height,
				header_head.last_block_h,
				txhashset_head.height,
				bhash
			);
			if let Err(e) = peer.send_txhashset_request(txhashset_head.height, bhash) {
				error!(LOGGER, "fast_sync: send_txhashset_request err! {:?}", e);
				return Err(e);
			}
			return Ok(peer.clone());
		}
		Err(p2p::Error::PeerException)
	}

	// For now this is a one-time thing (it can be slow) at initial startup.
	fn fast_sync_due(&mut self) -> (bool, bool) {
		let now = Utc::now();
		let mut download_timeout = false;

		match self.prev_fast_sync {
			None => {
				self.prev_fast_sync = Some(now);
				(true, download_timeout)
			}
			Some(prev) => {
				if now - prev > Duration::minutes(10) {
					download_timeout = true;
				}
				(false, download_timeout)
			}
		}
	}

	fn fast_sync_reset(&mut self) {
		self.prev_fast_sync = None;
		self.fast_sync_peer = None;
	}
}
