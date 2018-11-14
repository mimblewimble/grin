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
use std::cmp;
use std::sync::Arc;

use chain;
use common::types::{SyncState, SyncStatus};
use core::core::hash::Hash;
use p2p;

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
	pub fn check_run(&mut self, head: &chain::Tip, highest_height: u64) -> bool {
		// run the body_sync every 5s
		if self.body_sync_due() {
			if self.body_sync() {
				return true;
			}

			self.sync_state.update(SyncStatus::BodySync {
				current_height: head.height,
				highest_height: highest_height,
			});
		}
		false
	}

	/// Return true if txhashset download is needed (when requested block is under the horizon).
	fn body_sync(&mut self) -> bool {
		let mut hashes: Option<Vec<Hash>> = Some(vec![]);
		if self
			.chain
			.check_txhashset_needed("body_sync".to_owned(), &mut hashes)
		{
			debug!(
				"body_sync: cannot sync full blocks earlier than horizon. will request txhashset",
			);
			return true;
		}

		let mut hashes = hashes.unwrap();
		hashes.reverse();

		let peers = self.peers.more_work_peers();

		// if we have 5 peers to sync from then ask for 50 blocks total (peer_count *
		// 10) max will be 80 if all 8 peers are advertising more work
		// also if the chain is already saturated with orphans, throttle
		let block_count = cmp::min(
			cmp::min(100, peers.len() * p2p::SEND_CHANNEL_CAP),
			chain::MAX_ORPHAN_SIZE.saturating_sub(self.chain.orphans_len()) + 1,
		);

		let hashes_to_get = hashes
			.iter()
			.filter(|x| {
				// only ask for blocks that we have not yet processed
				// either successfully stored or in our orphan list
				!self.chain.get_block(x).is_ok() && !self.chain.is_orphan(x)
			}).take(block_count)
			.collect::<Vec<_>>();

		if hashes_to_get.len() > 0 {
			let body_head = self.chain.head().unwrap();
			let header_head = self.chain.header_head().unwrap();

			debug!(
				"block_sync: {}/{} requesting blocks {:?} from {} peers",
				body_head.height,
				header_head.height,
				hashes_to_get,
				peers.len(),
			);

			// reinitialize download tracking state
			self.blocks_requested = 0;
			self.receive_timeout = Utc::now() + Duration::seconds(6);

			let mut peers_iter = peers.iter().cycle();
			for hash in hashes_to_get.clone() {
				if let Some(peer) = peers_iter.next() {
					if let Err(e) = peer.send_block_request(*hash) {
						debug!("Skipped request to {}: {:?}", peer.info.addr, e);
					} else {
						self.blocks_requested += 1;
					}
				}
			}
		}
		return false;
	}

	// Should we run block body sync and ask for more full blocks?
	fn body_sync_due(&mut self) -> bool {
		let blocks_received = self.blocks_received();

		// some blocks have been requested
		if self.blocks_requested > 0 {
			// but none received since timeout, ask again
			let timeout = Utc::now() > self.receive_timeout;
			if timeout && blocks_received <= self.prev_blocks_received {
				debug!(
					"body_sync: expecting {} more blocks and none received for a while",
					self.blocks_requested,
				);
				return true;
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
			debug!("body_sync: no pending block request, asking more");
			return true;
		}

		return false;
	}

	// Total numbers received on this chain, including the head and orphans
	fn blocks_received(&self) -> u64 {
		self.chain.head().unwrap().height
			+ self.chain.orphans_len() as u64
			+ self.chain.orphans_evicted_len() as u64
	}
}
