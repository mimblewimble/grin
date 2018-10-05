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
use core::core::hash::{Hash, Hashed};
use p2p::{self, Peer};
use util::LOGGER;

pub struct HeaderSync {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,

	history_locators: Vec<(u64, Hash)>,
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
			history_locators: vec![],
			prev_header_sync: (Utc::now(), 0, 0),
		}
	}

	pub fn check_run(&mut self, header_head: &chain::Tip, highest_height: u64) -> bool {
		if !self.header_sync_due(header_head) {
			return false;
		}

		let status = self.sync_state.status();

		let enable_header_sync = match status {
			SyncStatus::BodySync { .. } | SyncStatus::HeaderSync { .. } => true,
			SyncStatus::NoSync | SyncStatus::Initial => {
				// Reset sync_head to header_head on transition to HeaderSync,
				// but ONLY on initial transition to HeaderSync state.
				let sync_head = self.chain.get_sync_head().unwrap();
				debug!(
					LOGGER,
					"sync: initial transition to HeaderSync. sync_head: {} at {}, reset to: {} at {}",
					sync_head.hash(),
					sync_head.height,
					header_head.hash(),
					header_head.height,
				);
				self.chain.reset_sync_head(&header_head).unwrap();
				self.history_locators.clear();
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
			SyncStatus::NoSync | SyncStatus::Initial => true,
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
				if let Ok(p) = peer.try_read() {
					let peer_difficulty = p.info.total_difficulty.clone();
					if peer_difficulty > difficulty {
						self.request_headers(&p);
					}
				}
			}
		}
	}

	/// Request some block headers from a peer to advance us.
	fn request_headers(&mut self, peer: &Peer) {
		if let Ok(locator) = self.get_locator() {
			debug!(
				LOGGER,
				"sync: request_headers: asking {} for headers, {:?}", peer.info.addr, locator,
			);

			let _ = peer.send_header_request(locator);
		}
	}

	/// We build a locator based on sync_head.
	/// Even if sync_head is significantly out of date we will "reset" it once we
	/// start getting headers back from a peer.
	fn get_locator(&mut self) -> Result<Vec<Hash>, Error> {
		let mut this_height = 0;

		let tip = self.chain.get_sync_head()?;
		let heights = get_locator_heights(tip.height);
		let mut new_heights: Vec<u64> = vec![];

		// for security, clear history_locators[] in any case of header chain rollback,
		// the easiest way is to check whether the sync head and the header head are identical.
		if self.history_locators.len() > 0 && tip.hash() != self.chain.header_head()?.hash() {
			self.history_locators.clear();
		}

		debug!(LOGGER, "sync: locator heights : {:?}", heights);

		let mut locator: Vec<Hash> = vec![];
		let mut current = self.chain.get_block_header(&tip.last_block_h);
		while let Ok(header) = current {
			if heights.contains(&header.height) {
				locator.push(header.hash());
				new_heights.push(header.height);
				if self.history_locators.len() > 0
					&& tip.height - header.height + 1 >= p2p::MAX_BLOCK_HEADERS as u64 - 1
				{
					this_height = header.height;
					break;
				}
			}
			current = self.chain.get_block_header(&header.previous);
		}

		// update history locators
		{
			let mut tmp: Vec<(u64, Hash)> = vec![];
			*&mut tmp = new_heights
				.clone()
				.into_iter()
				.zip(locator.clone().into_iter())
				.collect();
			tmp.reverse();
			if self.history_locators.len() > 0 && tmp[0].0 == 0 {
				tmp = tmp[1..].to_vec();
			}
			self.history_locators.append(&mut tmp);
		}

		// reuse remaining part of locator from history
		if this_height > 0 {
			let this_height_index = heights.iter().position(|&r| r == this_height).unwrap();
			let next_height = heights[this_height_index + 1];

			let reuse_index = self
				.history_locators
				.iter()
				.position(|&r| r.0 >= next_height)
				.unwrap();
			let mut tmp = self.history_locators[..reuse_index + 1].to_vec();
			tmp.reverse();
			for (height, hash) in &mut tmp {
				if *height == 0 {
					break;
				}

				// check the locator to make sure the gap >= 2^n, where n = index of heights Vec
				if this_height >= *height + 2u64.pow(locator.len() as u32) {
					locator.push(hash.clone());
					this_height = *height;
					new_heights.push(this_height);
				}
				if locator.len() >= (p2p::MAX_LOCATORS as usize) - 1 {
					break;
				}
			}

			// push height 0 if it's not there
			if new_heights[new_heights.len() - 1] != 0 {
				locator.push(
					self.history_locators[self.history_locators.len() - 1]
						.1
						.clone(),
				);
				new_heights.push(0);
			}
		}

		debug!(LOGGER, "sync: locator heights': {:?}", new_heights);

		// shrink history_locators properly
		if heights.len() > 1 {
			let shrink_height = heights[heights.len() - 2];
			let mut shrunk_size = 0;
			let shrink_index = self
				.history_locators
				.iter()
				.position(|&r| r.0 > shrink_height)
				.unwrap();
			if shrink_index > 100 {
				// shrink but avoid trivial shrinking
				let mut shrunk = self.history_locators[shrink_index..].to_vec();
				shrunk_size = shrink_index;
				self.history_locators.clear();
				self.history_locators.push((0, locator[locator.len() - 1]));
				self.history_locators.append(&mut shrunk);
			}
			debug!(
				LOGGER,
				"sync: history locators: len={}, shrunk={}",
				self.history_locators.len(),
				shrunk_size
			);
		}

		debug!(LOGGER, "sync: locator: {:?}", locator);

		Ok(locator)
	}
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
}
