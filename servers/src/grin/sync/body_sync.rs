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
use p2p::Capabilities;
use rand::prelude::*;
use std::cmp;
use std::sync::Arc;

use crate::chain::{self, SyncState, SyncStatus, Tip};
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::BlockHeader;
use crate::p2p;

pub struct BodySync {
	chain: Arc<chain::Chain>,
	peers: Arc<p2p::Peers>,
	sync_state: Arc<SyncState>,

	blocks_requested: u64,

	receive_timeout: DateTime<Utc>,
	prev_blocks_received: u64,
}

impl BodySync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
	) -> BodySync {
		BodySync {
			sync_state,
			peers,
			chain,
			blocks_requested: 0,
			receive_timeout: Utc::now(),
			prev_blocks_received: 0,
		}
	}

	/// Check whether a body sync is needed and run it if so.
	/// Return true if txhashset download is needed (when requested block is under the horizon).
	pub fn check_run(
		&mut self,
		head: &chain::Tip,
		highest_height: u64,
	) -> Result<bool, chain::Error> {
		// run the body_sync every 5s
		if self.body_sync_due()? {
			if self.body_sync()? {
				return Ok(true);
			}

			self.sync_state.update(SyncStatus::BodySync {
				current_height: head.height,
				highest_height: highest_height,
			});
		}
		Ok(false)
	}

	/// Is our local node running in archive_mode?
	fn archive_mode(&self) -> bool {
		self.chain.archive_mode()
	}

	/// Return true if txhashset download is needed (when requested block is under the horizon).
	/// Otherwise go request some missing blocks and return false.
	fn body_sync(&mut self) -> Result<bool, chain::Error> {
		let head = self.chain.head()?;
		let header_head = self.chain.header_head()?;
		let fork_point = self.chain.fork_point()?;

		if self.chain.check_txhashset_needed(&fork_point)? {
			trace!(
				"body_sync: cannot sync full blocks earlier than horizon. will request txhashset",
			);
			return Ok(true);
		}

		let peers = {
			// Find connected peers with strictly greater difficulty than us.
			let peers_iter = || {
				// If we are running with archive mode enabled we only want to sync
				// from other archive nodes.
				let cap = if self.archive_mode() {
					Capabilities::BLOCK_HIST
				} else {
					Capabilities::UNKNOWN
				};

				self.peers
					.iter()
					.with_capabilities(cap)
					.with_difficulty(|x| x > head.total_difficulty)
					.connected()
			};

			// We prefer outbound peers with greater difficulty.
			let mut peers: Vec<_> = peers_iter().outbound().into_iter().collect();
			if peers.is_empty() {
				debug!("no outbound peers with more work, considering inbound");
				peers = peers_iter().inbound().into_iter().collect();
			}

			// If we have no peers (outbound or inbound) then we are done for now.
			if peers.is_empty() {
				debug!("no peers (inbound or outbound) with more work");
				return Ok(false);
			}

			peers
		};

		// if we have 5 peers to sync from then ask for 50 blocks total (peer_count *
		// 10) max will be 80 if all 8 peers are advertising more work
		// also if the chain is already saturated with orphans, throttle
		let block_count = cmp::min(
			cmp::min(100, peers.len() * 10),
			chain::MAX_ORPHAN_SIZE.saturating_sub(self.chain.orphans_len()) + 1,
		);

		let hashes = self.block_hashes_to_sync(&fork_point, &header_head, block_count as u64)?;

		if !hashes.is_empty() {
			debug!(
				"block_sync: {}/{} requesting blocks {:?} from {} peers",
				head.height,
				header_head.height,
				hashes,
				peers.len(),
			);

			// reinitialize download tracking state
			self.blocks_requested = 0;
			self.receive_timeout = Utc::now() + Duration::seconds(6);

			let mut rng = rand::thread_rng();
			for hash in hashes {
				if let Some(peer) = peers.choose(&mut rng) {
					if let Err(e) = peer.send_block_request(hash, chain::Options::SYNC) {
						debug!("Skipped request to {}: {:?}", peer.info.addr, e);
						peer.stop();
					} else {
						self.blocks_requested += 1;
					}
				}
			}
		}
		return Ok(false);
	}

	fn block_hashes_to_sync(
		&self,
		fork_point: &BlockHeader,
		header_head: &Tip,
		count: u64,
	) -> Result<Vec<Hash>, chain::Error> {
		let mut hashes = vec![];
		let max_height = cmp::min(fork_point.height + count, header_head.height);
		let mut current = self.chain.get_header_by_height(max_height)?;
		while current.height > fork_point.height {
			if !self.chain.is_orphan(&current.hash()) {
				hashes.push(current.hash());
			}
			current = self.chain.get_previous_header(&current)?;
		}
		hashes.reverse();
		Ok(hashes)
	}

	// Should we run block body sync and ask for more full blocks?
	fn body_sync_due(&mut self) -> Result<bool, chain::Error> {
		let blocks_received = self.blocks_received()?;

		// some blocks have been requested
		if self.blocks_requested > 0 {
			// but none received since timeout, ask again
			let timeout = Utc::now() > self.receive_timeout;
			if timeout && blocks_received <= self.prev_blocks_received {
				debug!(
					"body_sync: expecting {} more blocks and none received for a while",
					self.blocks_requested,
				);
				return Ok(true);
			}
		}

		if blocks_received > self.prev_blocks_received {
			// some received, update for next check
			self.receive_timeout = Utc::now() + Duration::seconds(1);
			self.blocks_requested = self
				.blocks_requested
				.saturating_sub(blocks_received - self.prev_blocks_received);
			self.prev_blocks_received = blocks_received;
		}

		// off by one to account for broadcast adding a couple orphans
		if self.blocks_requested < 2 {
			// no pending block requests, ask more
			trace!("body_sync: no pending block request, asking more");
			return Ok(true);
		}

		Ok(false)
	}

	// Total numbers received on this chain, including the head and orphans
	fn blocks_received(&self) -> Result<u64, chain::Error> {
		Ok((self.chain.head()?).height
			+ self.chain.orphans_len() as u64
			+ self.chain.orphans_evicted_len() as u64)
	}
}
