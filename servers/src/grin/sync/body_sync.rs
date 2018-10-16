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
use core::core::hash::{Hash, Hashed, ZERO_HASH};
use core::global;
use p2p;
use util::LOGGER;

pub struct BodySync {
	chain: Arc<chain::Chain>,
	peers: Arc<p2p::Peers>,
	sync_state: Arc<SyncState>,

	prev_body_sync: (DateTime<Utc>, u64),
	sync_start_ts: DateTime<Utc>,
	body_sync_hashes: Vec<Hash>,
	prev_body_received: Option<DateTime<Utc>>,
	prev_tip: chain::Tip,
	prev_orphans_len: usize,
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
			prev_body_sync: (Utc::now(), 0),
			sync_start_ts: Utc::now(),
			body_sync_hashes: vec![],
			prev_body_received: None,
			prev_tip: chain::Tip::new(ZERO_HASH),
			prev_orphans_len: 0,
		}
	}

	/// Check whether a body sync is needed and run it if so
	pub fn check_run(&mut self, head: &chain::Tip, highest_height: u64) -> bool {
		// if fast_sync disabled or not needed, run the body_sync every 5s
		if self.body_sync_due(head) {
			self.body_sync();

			self.sync_state.update(SyncStatus::BodySync {
				current_height: head.height,
				highest_height: highest_height,
			});
			return true;
		}
		false
	}

	fn body_sync_due(&mut self, head: &chain::Tip) -> bool {
		let now = Utc::now();
		let (prev_ts, prev_height) = self.prev_body_sync;

		if head.height >= prev_height + 96
			|| now - prev_ts > Duration::seconds(5)
			|| self.block_batch_received()
		{
			self.prev_body_sync = (now, head.height);
			return true;
		}
		false
	}

	fn body_sync(&mut self) {
		let horizon = global::cut_through_horizon() as u64;
		let body_head: chain::Tip = self.chain.head().unwrap();
		let header_head: chain::Tip = self.chain.header_head().unwrap();
		let sync_head: chain::Tip = self.chain.get_sync_head().unwrap();

		self.reset();

		debug!(
			LOGGER,
			"body_sync: body_head - {}, {}, header_head - {}, {}, sync_head - {}, {}",
			body_head.last_block_h,
			body_head.height,
			header_head.last_block_h,
			header_head.height,
			sync_head.last_block_h,
			sync_head.height,
		);

		let mut hashes = vec![];
		let mut oldest_height = 0;

		if header_head.total_difficulty > body_head.total_difficulty {
			let mut current = self.chain.get_block_header(&header_head.last_block_h);
			while let Ok(header) = current {
				// break out of the while loop when we find a header common
				// between the header chain and the current body chain
				if let Ok(_) = self.chain.is_on_current_chain(&header) {
					break;
				}

				hashes.push(header.hash());
				oldest_height = header.height;
				current = self.chain.get_block_header(&header.previous);
			}
		}
		hashes.reverse();

		// if we have 5 peers to sync from then ask for 50 blocks total (peer_count *
		// 10) max will be 80 if all 8 peers are advertising more work
		// also if the chain is already saturated with orphans, throttle
		let peers = if oldest_height < header_head.height.saturating_sub(horizon) {
			self.peers.more_work_archival_peers()
		} else {
			self.peers.more_work_peers()
		};

		let block_count = cmp::min(
			cmp::min(100, peers.len() * 10),
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
			debug!(
				LOGGER,
				"block_sync: {}/{} requesting blocks {:?} from {} peers",
				body_head.height,
				header_head.height,
				hashes_to_get,
				peers.len(),
			);

			let mut peers_iter = peers.iter().cycle();

			for hash in hashes_to_get.clone() {
				if let Some(peer) = peers_iter.next() {
					if let Err(e) = peer.send_block_request(*hash) {
						debug!(LOGGER, "Skipped request to {}: {:?}", peer.info.addr, e);
					} else {
						self.body_sync_hashes.push(hash.clone());
					}
				}
			}
		}

		self.reset_start();
	}

	fn reset(&mut self) {
		self.body_sync_hashes.clear();
		self.prev_body_received = None;
	}

	fn reset_start(&mut self) {
		self.prev_tip = self.chain.head().unwrap();
		self.prev_orphans_len = self.chain.orphans_len() + self.chain.orphans_evicted_len();
		self.sync_start_ts = Utc::now();
	}

	fn block_batch_received(&mut self) -> bool {
		let tip = self.chain.head().unwrap();

		match self.prev_body_received {
			Some(prev_ts) => {
				if tip.last_block_h == self.prev_tip.last_block_h
					&& self.chain.orphans_len() + self.chain.orphans_evicted_len()
						== self.prev_orphans_len
					&& Utc::now() - prev_ts > Duration::milliseconds(200)
				{
					let hashes_not_get = self
						.body_sync_hashes
						.iter()
						.filter(|x| !self.chain.get_block(*x).is_ok() && !self.chain.is_orphan(*x))
						.collect::<Vec<_>>();
					debug!(
						LOGGER,
						"body_sync: {}/{} blocks received, and no more in 200ms",
						self.body_sync_hashes.len() - hashes_not_get.len(),
						self.body_sync_hashes.len(),
					);
					return true;
				}
			}
			None => {
				if Utc::now() - self.sync_start_ts > Duration::seconds(5) {
					debug!(
						LOGGER,
						"body_sync: 0/{} blocks received in 5s",
						self.body_sync_hashes.len(),
					);
					return true;
				}
			}
		}

		if tip.last_block_h != self.prev_tip.last_block_h
			|| self.chain.orphans_len() + self.chain.orphans_evicted_len() != self.prev_orphans_len
		{
			self.prev_tip = tip;
			self.prev_body_received = Some(Utc::now());
			self.prev_orphans_len = self.chain.orphans_len() + self.chain.orphans_evicted_len();
		}

		return false;
	}
}
