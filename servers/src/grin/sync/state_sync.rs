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

use chrono::prelude::{DateTime, Utc};
use chrono::Duration;
use std::sync::Arc;

use crate::chain::{self, SyncState, SyncStatus};
use crate::core::core::hash::Hashed;
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::p2p::{self, Capabilities, Peer};

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

	prev_state_sync: Option<DateTime<Utc>>,
	state_sync_peer: Option<Arc<Peer>>,
}

impl StateSync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
	) -> StateSync {
		StateSync {
			sync_state,
			peers,
			chain,
			prev_state_sync: None,
			state_sync_peer: None,
		}
	}

	/// Check whether state sync should run and triggers a state download when
	/// it's time (we have all headers). Returns true as long as state sync
	/// needs monitoring, false when it's either done or turned off.
	pub fn check_run(
		&mut self,
		header_head: &chain::Tip,
		head: &chain::Tip,
		tail: &chain::Tip,
		highest_height: u64,
	) -> bool {
		trace!("state_sync: head.height: {}, tail.height: {}. header_head.height: {}, highest_height: {}",
			   head.height, tail.height, header_head.height, highest_height,
		);

		let mut sync_need_restart = false;

		// check sync error
		if let Some(sync_error) = self.sync_state.sync_error() {
			error!("state_sync: error = {}. restart fast sync", sync_error);
			sync_need_restart = true;
		}

		// check peer connection status of this sync
		if let Some(ref peer) = self.state_sync_peer {
			if let SyncStatus::TxHashsetDownload { .. } = self.sync_state.status() {
				if !peer.is_connected() {
					sync_need_restart = true;
					info!(
						"state_sync: peer connection lost: {:?}. restart",
						peer.info.addr,
					);
				}
			}
		}

		// if txhashset downloaded and validated successfully, we switch to BodySync state,
		// and we need call state_sync_reset() to make it ready for next possible state sync.
		let done = self.sync_state.update_if(
			SyncStatus::BodySync {
				current_height: 0,
				highest_height: 0,
			},
			|s| match s {
				SyncStatus::TxHashsetDone => true,
				_ => false,
			},
		);

		if sync_need_restart || done {
			self.state_sync_reset();
			self.sync_state.clear_sync_error();
		}

		if done {
			return false;
		}

		// run fast sync if applicable, normally only run one-time, except restart in error
		if sync_need_restart || header_head.height == highest_height {
			let (go, download_timeout) = self.state_sync_due();

			if let SyncStatus::TxHashsetDownload { .. } = self.sync_state.status() {
				if download_timeout {
					error!("state_sync: TxHashsetDownload status timeout in 10 minutes!");
					self.sync_state.set_sync_error(
						chain::ErrorKind::SyncError(format!("{:?}", p2p::Error::Timeout)).into(),
					);
				}
			}

			if go {
				self.state_sync_peer = None;
				match self.request_state(&header_head) {
					Ok(peer) => {
						self.state_sync_peer = Some(peer);
					}
					Err(e) => self
						.sync_state
						.set_sync_error(chain::ErrorKind::SyncError(format!("{:?}", e)).into()),
				}

				// to avoid the confusing log,
				// update the final HeaderSync state mainly for 'current_height'
				self.sync_state.update_if(
					SyncStatus::HeaderSync {
						current_height: header_head.height,
						highest_height,
					},
					|s| match s {
						SyncStatus::HeaderSync { .. } => true,
						_ => false,
					},
				);

				self.sync_state
					.update(SyncStatus::TxHashsetDownload(Default::default()));
			}
		}
		true
	}

	fn request_state(&self, header_head: &chain::Tip) -> Result<Arc<Peer>, p2p::Error> {
		let threshold = global::state_sync_threshold() as u64;
		let archive_interval = global::txhashset_archive_interval();
		let mut txhashset_height = header_head.height.saturating_sub(threshold);
		txhashset_height = txhashset_height.saturating_sub(txhashset_height % archive_interval);

		let peers_iter = || {
			self.peers
				.iter()
				.with_capabilities(Capabilities::TXHASHSET_HIST)
				.connected()
		};

		// Filter peers further based on max difficulty.
		let max_diff = peers_iter().max_difficulty().unwrap_or(Difficulty::zero());
		let peers_iter = || peers_iter().with_difficulty(|x| x >= max_diff);

		// Choose a random "most work" peer, preferring outbound if at all possible.
		let peer = peers_iter().outbound().choose_random().or_else(|| {
			warn!("no suitable outbound peer for state sync, considering inbound");
			peers_iter().inbound().choose_random()
		});

		if let Some(peer) = peer {
			// ask for txhashset at state_sync_threshold
			let mut txhashset_head = self
				.chain
				.get_block_header(&header_head.prev_block_h)
				.map_err(|e| {
					error!(
						"chain error during getting a block header {}: {:?}",
						&header_head.prev_block_h, e
					);
					p2p::Error::Internal
				})?;
			while txhashset_head.height > txhashset_height {
				txhashset_head = self
					.chain
					.get_previous_header(&txhashset_head)
					.map_err(|e| {
						error!(
							"chain error during getting a previous block header {}: {:?}",
							txhashset_head.hash(),
							e
						);
						p2p::Error::Internal
					})?;
			}
			let bhash = txhashset_head.hash();
			debug!(
				"state_sync: before txhashset request, header head: {} / {}, txhashset_head: {} / {}",
				header_head.height,
				header_head.last_block_h,
				txhashset_head.height,
				bhash
			);
			if let Err(e) = peer.send_txhashset_request(txhashset_head.height, bhash) {
				error!("state_sync: send_txhashset_request err! {:?}", e);
				return Err(e);
			}
			return Ok(peer);
		}
		Err(p2p::Error::PeerException)
	}

	// For now this is a one-time thing (it can be slow) at initial startup.
	fn state_sync_due(&mut self) -> (bool, bool) {
		let now = Utc::now();
		let mut download_timeout = false;

		match self.prev_state_sync {
			None => {
				self.prev_state_sync = Some(now);
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

	fn state_sync_reset(&mut self) {
		self.prev_state_sync = None;
		self.state_sync_peer = None;
	}
}
