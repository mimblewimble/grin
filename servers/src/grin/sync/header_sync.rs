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

use crate::chain;
use crate::common::types::{Error, SyncState, SyncStatus};
use crate::core::core::hash::{Hash, Hashed};
use crate::p2p::{self, Peer};

pub struct HeaderSync {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,

	history_locator: Vec<(u64, Hash)>,
	prev_header_sync: (DateTime<Utc>, u64, u64),
}

impl HeaderSync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
	) -> HeaderSync {
		HeaderSync {
			sync_state,
			peers,
			chain,
			history_locator: vec![],
			prev_header_sync: (Utc::now(), 0, 0),
		}
	}

	pub fn check_run(&mut self, header_head: &chain::Tip, highest_height: u64) -> bool {
		if !self.header_sync_due(header_head) {
			return false;
		}

		let enable_header_sync = match self.sync_state.status() {
			SyncStatus::BodySync { .. }
			| SyncStatus::HeaderSync { .. }
			| SyncStatus::TxHashsetDone => true,
			SyncStatus::NoSync | SyncStatus::Initial | SyncStatus::AwaitingPeers(_) => {
				// Reset sync_head to header_head on transition to HeaderSync,
				// but ONLY on initial transition to HeaderSync state.
				let sync_head = self.chain.get_sync_head().unwrap();
				debug!(
					"sync: initial transition to HeaderSync. sync_head: {} at {}, reset to: {} at {}",
					sync_head.hash(),
					sync_head.height,
					header_head.hash(),
					header_head.height,
				);

				// Rebuild the sync MMR to match our updated sync_head.
				self.chain.rebuild_sync_mmr(&header_head).unwrap();

				self.history_locator.retain(|&x| x.0 == 0);
				true
			}
			_ => false,
		};

		if enable_header_sync {
			self.sync_state.update(SyncStatus::HeaderSync {
				current_height: header_head.height,
				highest_height: highest_height,
			});

			self.header_sync();
			return true;
		}
		false
	}

	fn header_sync_due(&mut self, header_head: &chain::Tip) -> bool {
		let now = Utc::now();
		let (timeout, latest_height, prev_height) = self.prev_header_sync;

		// received all necessary headers, can ask for more
		let all_headers_received =
			header_head.height >= prev_height + (p2p::MAX_BLOCK_HEADERS as u64) - 4;
		// no headers processed and we're past timeout, need to ask for more
		let stalling = header_head.height <= latest_height && now > timeout;

		// always enable header sync on initial state transition from NoSync / Initial
		let force_sync = match self.sync_state.status() {
			SyncStatus::NoSync | SyncStatus::Initial | SyncStatus::AwaitingPeers(_) => true,
			_ => false,
		};

		if force_sync || all_headers_received || stalling {
			self.prev_header_sync = (
				now + Duration::seconds(10),
				header_head.height,
				header_head.height,
			);
			true
		} else {
			// resetting the timeout as long as we progress
			if header_head.height > latest_height {
				self.prev_header_sync =
					(now + Duration::seconds(2), header_head.height, prev_height);
			}
			false
		}
	}

	fn header_sync(&mut self) {
		if let Ok(header_head) = self.chain.header_head() {
			let difficulty = header_head.total_difficulty;

			if let Some(peer) = self.peers.most_work_peer() {
				if peer.info.total_difficulty() > difficulty {
					self.request_headers(&peer);
				}
			}
		}
	}

	/// Request some block headers from a peer to advance us.
	fn request_headers(&mut self, peer: &Peer) {
		if let Ok(locator) = self.get_locator() {
			debug!(
				"sync: request_headers: asking {} for headers, {:?}",
				peer.info.addr, locator,
			);

			let _ = peer.send_header_request(locator);
		}
	}

	/// We build a locator based on sync_head.
	/// Even if sync_head is significantly out of date we will "reset" it once we
	/// start getting headers back from a peer.
	fn get_locator(&mut self) -> Result<Vec<Hash>, Error> {
		let tip = self.chain.get_sync_head()?;
		let heights = get_locator_heights(tip.height);

		// for security, clear history_locator[] in any case of header chain rollback,
		// the easiest way is to check whether the sync head and the header head are identical.
		if self.history_locator.len() > 0 && tip.hash() != self.chain.header_head()?.hash() {
			self.history_locator.retain(|&x| x.0 == 0);
		}

		// for each height we need, we either check if something is close enough from
		// last locator, or go to the db
		let mut locator: Vec<(u64, Hash)> = vec![(tip.height, tip.last_block_h)];
		for h in heights {
			if let Some(l) = close_enough(&self.history_locator, h) {
				locator.push(l);
			} else {
				// start at last known hash and go backward
				let last_loc = locator.last().unwrap().clone();
				let mut header_cursor = self.chain.get_block_header(&last_loc.1);
				while let Ok(header) = header_cursor {
					if header.height == h {
						if header.height != last_loc.0 {
							locator.push((header.height, header.hash()));
						}
						break;
					}
					header_cursor = self.chain.get_previous_header(&header);
				}
			}
		}
		locator.dedup_by(|a, b| a.0 == b.0);
		debug!("sync: locator : {:?}", locator.clone());
		self.history_locator = locator.clone();

		Ok(locator.iter().map(|l| l.1).collect())
	}
}

// Whether we have a value close enough to the provided height in the locator
fn close_enough(locator: &Vec<(u64, Hash)>, height: u64) -> Option<(u64, Hash)> {
	if locator.len() == 0 {
		return None;
	}
	// bounds, lower that last is last
	if locator.last().unwrap().0 >= height {
		return locator.last().map(|l| l.clone());
	}
	// higher than first is first if within an acceptable gap
	if locator[0].0 < height && height.saturating_sub(127) < locator[0].0 {
		return Some(locator[0]);
	}
	for hh in locator.windows(2) {
		if height <= hh[0].0 && height > hh[1].0 {
			if hh[0].0 - height < height - hh[1].0 {
				return Some(hh[0].clone());
			} else {
				return Some(hh[1].clone());
			}
		}
	}
	None
}

// current height back to 0 decreasing in powers of 2
fn get_locator_heights(height: u64) -> Vec<u64> {
	let mut current = height;
	let mut heights = vec![];
	while current > 0 {
		heights.push(current);
		if heights.len() >= (p2p::MAX_LOCATORS as usize) - 1 {
			break;
		}
		let next = 2u64.pow(heights.len() as u32);
		current = if current > next { current - next } else { 0 }
	}
	heights.push(0);
	heights
}

#[cfg(test)]
mod test {
	use super::*;
	use crate::core::core::hash;

	#[test]
	fn test_get_locator_heights() {
		assert_eq!(get_locator_heights(0), vec![0]);
		assert_eq!(get_locator_heights(1), vec![1, 0]);
		assert_eq!(get_locator_heights(2), vec![2, 0]);
		assert_eq!(get_locator_heights(3), vec![3, 1, 0]);
		assert_eq!(get_locator_heights(10), vec![10, 8, 4, 0]);
		assert_eq!(get_locator_heights(100), vec![100, 98, 94, 86, 70, 38, 0]);
		assert_eq!(
			get_locator_heights(1000),
			vec![1000, 998, 994, 986, 970, 938, 874, 746, 490, 0]
		);
		// check the locator is still a manageable length, even for large numbers of
		// headers
		assert_eq!(
			get_locator_heights(10000),
			vec![10000, 9998, 9994, 9986, 9970, 9938, 9874, 9746, 9490, 8978, 7954, 5906, 1810, 0,]
		);
	}

	#[test]
	fn test_close_enough() {
		let zh = hash::ZERO_HASH;

		// empty check
		assert_eq!(close_enough(&vec![], 0), None);

		// just 1 locator in history
		let heights: Vec<u64> = vec![64, 62, 58, 50, 34, 2, 0];
		let history_locator: Vec<(u64, Hash)> = vec![(0, zh.clone())];
		let mut locator: Vec<(u64, Hash)> = vec![];
		for h in heights {
			if let Some(l) = close_enough(&history_locator, h) {
				locator.push(l);
			}
		}
		assert_eq!(locator, vec![(0, zh.clone())]);

		// simple dummy example
		let locator = vec![
			(1000, zh.clone()),
			(500, zh.clone()),
			(250, zh.clone()),
			(125, zh.clone()),
		];
		assert_eq!(close_enough(&locator, 2000), None);
		assert_eq!(close_enough(&locator, 1050), Some((1000, zh)));
		assert_eq!(close_enough(&locator, 900), Some((1000, zh)));
		assert_eq!(close_enough(&locator, 270), Some((250, zh)));
		assert_eq!(close_enough(&locator, 20), Some((125, zh)));
		assert_eq!(close_enough(&locator, 125), Some((125, zh)));
		assert_eq!(close_enough(&locator, 500), Some((500, zh)));

		// more realistic test with 11 history
		let heights: Vec<u64> = vec![
			2554, 2552, 2548, 2540, 2524, 2492, 2428, 2300, 2044, 1532, 508, 0,
		];
		let history_locator: Vec<(u64, Hash)> = vec![
			(2043, zh.clone()),
			(2041, zh.clone()),
			(2037, zh.clone()),
			(2029, zh.clone()),
			(2013, zh.clone()),
			(1981, zh.clone()),
			(1917, zh.clone()),
			(1789, zh.clone()),
			(1532, zh.clone()),
			(1021, zh.clone()),
			(0, zh.clone()),
		];
		let mut locator: Vec<(u64, Hash)> = vec![];
		for h in heights {
			if let Some(l) = close_enough(&history_locator, h) {
				locator.push(l);
			}
		}
		locator.dedup_by(|a, b| a.0 == b.0);
		assert_eq!(
			locator,
			vec![(2043, zh.clone()), (1532, zh.clone()), (0, zh.clone()),]
		);

		// more realistic test with 12 history
		let heights: Vec<u64> = vec![
			4598, 4596, 4592, 4584, 4568, 4536, 4472, 4344, 4088, 3576, 2552, 504, 0,
		];
		let history_locator: Vec<(u64, Hash)> = vec![
			(4087, zh.clone()),
			(4085, zh.clone()),
			(4081, zh.clone()),
			(4073, zh.clone()),
			(4057, zh.clone()),
			(4025, zh.clone()),
			(3961, zh.clone()),
			(3833, zh.clone()),
			(3576, zh.clone()),
			(3065, zh.clone()),
			(1532, zh.clone()),
			(0, zh.clone()),
		];
		let mut locator: Vec<(u64, Hash)> = vec![];
		for h in heights {
			if let Some(l) = close_enough(&history_locator, h) {
				locator.push(l);
			}
		}
		locator.dedup_by(|a, b| a.0 == b.0);
		assert_eq!(
			locator,
			vec![
				(4087, zh.clone()),
				(3576, zh.clone()),
				(3065, zh.clone()),
				(0, zh.clone()),
			]
		);
	}
}
