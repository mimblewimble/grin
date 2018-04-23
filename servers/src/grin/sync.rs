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

use std::{cmp, thread};
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use time;

use chain;
use common::types::Error;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::global;
use grin::sync;
use p2p::{self, Peer, Peers};
use util::LOGGER;

pub struct Syncer {}

impl Syncer {
	pub fn new() -> Syncer {
		Syncer {}
	}

	pub fn run_sync(
		&self,
		currently_syncing: Arc<AtomicBool>,
		awaiting_peers: Arc<AtomicBool>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
		skip_sync_wait: bool,
		archive_mode: bool,
		stop: Arc<AtomicBool>,
	) {
		sync::run_sync(
			currently_syncing,
			awaiting_peers,
			peers,
			chain,
			skip_sync_wait,
			archive_mode,
			stop,
		)
	}
}

/// Starts the syncing loop, just spawns two threads that loop forever
pub fn run_sync(
	currently_syncing: Arc<AtomicBool>,
	awaiting_peers: Arc<AtomicBool>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	skip_sync_wait: bool,
	archive_mode: bool,
	stop: Arc<AtomicBool>,
) {
	let chain = chain.clone();
	let _ = thread::Builder::new()
		.name("sync".to_string())
		.spawn(move || {
			let mut si = SyncInfo::new();

			// initial sleep to give us time to peer with some nodes
			if !skip_sync_wait {
				awaiting_peers.store(true, Ordering::Relaxed);
				let mut n = 0;
				while peers.more_work_peers().len() < 4 && n < 30 {
					thread::sleep(Duration::from_secs(1));
					n += 1;
				}
				awaiting_peers.store(false, Ordering::Relaxed);
			}

			// fast sync has 3 "states":
			// * syncing headers
			// * once all headers are sync'd, requesting the txhashset state
			// * once we have the state, get blocks after that
			//
			// full sync gets rid of the middle step and just starts from
			// the genesis state

			loop {
				let horizon = global::cut_through_horizon() as u64;
				let head = chain.head().unwrap();
				let header_head = chain.get_header_head().unwrap();

				// is syncing generally needed when we compare our state with others
				let (syncing, most_work_height) =
					needs_syncing(currently_syncing.as_ref(), peers.clone(), chain.clone());

				if most_work_height > 0 {
					// we can occasionally get a most work height of 0 if read locks fail
					si.highest_height = most_work_height;
				}

				if syncing {
					let fast_sync_enabled =
						!archive_mode && si.highest_height.saturating_sub(head.height) > horizon;

					// run the header sync every 10s
					if si.header_sync_due(&header_head) {
						header_sync(peers.clone(), chain.clone());
					}

					if fast_sync_enabled {
						// run fast sync if applicable, every 5 min
						if header_head.height == si.highest_height && si.fast_sync_due() {
							fast_sync(peers.clone(), chain.clone(), &header_head);
						}
					} else {
						// run the body_sync every 5s
						if si.body_sync_due(&head) {
							body_sync(peers.clone(), chain.clone());
						}
					}
				}

				currently_syncing.store(syncing, Ordering::Relaxed);

				thread::sleep(Duration::from_secs(1));

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});
}

fn body_sync(peers: Arc<Peers>, chain: Arc<chain::Chain>) {
	let body_head: chain::Tip = chain.head().unwrap();
	let header_head: chain::Tip = chain.get_header_head().unwrap();
	let sync_head: chain::Tip = chain.get_sync_head().unwrap();

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

	if header_head.total_difficulty > body_head.total_difficulty {
		let mut current = chain.get_block_header(&header_head.last_block_h);
		while let Ok(header) = current {
			// break out of the while loop when we find a header common
			// between the this chain and the current chain
			if let Ok(_) = chain.is_on_current_chain(&header) {
				break;
			}

			hashes.push(header.hash());
			current = chain.get_block_header(&header.previous);
		}
	}
	hashes.reverse();

	// if we have 5 peers to sync from then ask for 50 blocks total (peer_count *
	// 10) max will be 80 if all 8 peers are advertising more work
	let peer_count = cmp::min(peers.more_work_peers().len(), 10);
	let mut block_count = peer_count * 10;

	// if the chain is already saturated with orphans, throttle
	// still asking for at least 1 unknown block to avoid getting stuck
	block_count = cmp::min(
		block_count,
		chain::MAX_ORPHAN_SIZE.saturating_sub(chain.orphans_len()) + 1,
	);

	let hashes_to_get = hashes
		.iter()
		.filter(|x| {
			// only ask for blocks that we have not yet processed
			// either successfully stored or in our orphan list
			!chain.get_block(x).is_ok() && !chain.is_orphan(x)
		})
		.take(block_count)
		.cloned()
		.collect::<Vec<_>>();

	if hashes_to_get.len() > 0 {
		debug!(
			LOGGER,
			"block_sync: {}/{} requesting blocks {:?} from {} peers",
			body_head.height,
			header_head.height,
			hashes_to_get,
			peer_count,
		);

		for hash in hashes_to_get.clone() {
			// TODO - Is there a threshold where we sync from most_work_peer (not
			// more_work_peer)?
			let peer = peers.more_work_archival_peer();
			if let Some(peer) = peer {
				if let Ok(peer) = peer.try_read() {
					if let Err(e) = peer.send_block_request(hash) {
						debug!(LOGGER, "Skipped request to {}: {:?}", peer.info.addr, e);
					}
				}
			}
		}
	}
}

fn header_sync(peers: Arc<Peers>, chain: Arc<chain::Chain>) {
	if let Ok(header_head) = chain.get_header_head() {
		let difficulty = header_head.total_difficulty;

		if let Some(peer) = peers.most_work_peer() {
			if let Ok(p) = peer.try_read() {
				let peer_difficulty = p.info.total_difficulty.clone();
				if peer_difficulty > difficulty {
					request_headers(&p, chain.clone());
				}
			}
		}
	}
}

fn fast_sync(peers: Arc<Peers>, chain: Arc<chain::Chain>, header_head: &chain::Tip) {
	let horizon = global::cut_through_horizon() as u64;

	if let Some(peer) = peers.most_work_peer() {
		if let Ok(p) = peer.try_read() {
			debug!(
				LOGGER,
				"Header head before txhashset request: {} / {}",
				header_head.height,
				header_head.last_block_h
			);

			// ask for txhashset at horizon
			let mut txhashset_head = chain.get_block_header(&header_head.prev_block_h).unwrap();
			for _ in 0..horizon.saturating_sub(20) {
				txhashset_head = chain.get_block_header(&txhashset_head.previous).unwrap();
			}
			p.send_txhashset_request(txhashset_head.height, txhashset_head.hash())
				.unwrap();
		}
	}
}

/// Request some block headers from a peer to advance us.
fn request_headers(peer: &Peer, chain: Arc<chain::Chain>) {
	if let Ok(locator) = get_locator(chain) {
		debug!(
			LOGGER,
			"sync: request_headers: asking {} for headers, {:?}", peer.info.addr, locator,
		);

		let _ = peer.send_header_request(locator);
	}
}

/// Whether we're currently syncing the chain or we're fully caught up and
/// just receiving blocks through gossip.
fn needs_syncing(
	currently_syncing: &AtomicBool,
	peers: Arc<Peers>,
	chain: Arc<chain::Chain>,
) -> (bool, u64) {
	let local_diff = chain.total_difficulty();
	let peer = peers.most_work_peer();
	let is_syncing = currently_syncing.load(Ordering::Relaxed);
	let mut most_work_height = 0;

	// if we're already syncing, we're caught up if no peer has a higher
	// difficulty than us
	if is_syncing {
		if let Some(peer) = peer {
			if let Ok(peer) = peer.try_read() {
				most_work_height = peer.info.height;

				if peer.info.total_difficulty <= local_diff {
					let ch = chain.head().unwrap();
					info!(
						LOGGER,
						"synchronised at {} @ {} [{}]",
						local_diff.into_num(),
						ch.height,
						ch.last_block_h
					);

					let _ = chain.reset_head();
					return (false, 0);
				}
			}
		} else {
			warn!(LOGGER, "sync: no peers available, disabling sync");
			return (false, 0);
		}
	} else {
		if let Some(peer) = peer {
			if let Ok(peer) = peer.try_read() {
				most_work_height = peer.info.height;

				// sum the last 5 difficulties to give us the threshold
				let threshold = chain
					.difficulty_iter()
					.filter_map(|x| x.map(|(_, x)| x).ok())
					.take(5)
					.fold(Difficulty::zero(), |sum, val| sum + val);

				if peer.info.total_difficulty > local_diff.clone() + threshold.clone() {
					info!(
						LOGGER,
						"sync: total_difficulty {}, peer_difficulty {}, threshold {} (last 5 blocks), enabling sync",
						local_diff,
						peer.info.total_difficulty,
						threshold,
					);
					return (true, most_work_height);
				}
			}
		}
	}
	(is_syncing, most_work_height)
}

/// We build a locator based on sync_head.
/// Even if sync_head is significantly out of date we will "reset" it once we start getting
/// headers back from a peer.
///
/// TODO - this gets *expensive* with a large header chain to iterate over
/// as we need to get each block header from the db
/// can we add a get_block_header_by_height(height, hash) ???
///
fn get_locator(chain: Arc<chain::Chain>) -> Result<Vec<Hash>, Error> {
	let tip = chain.get_sync_head()?;
	let heights = get_locator_heights(tip.height);

	debug!(LOGGER, "sync: locator heights: {:?}", heights);

	let mut locator = vec![];
	let mut current = chain.get_block_header(&tip.last_block_h);
	while let Ok(header) = current {
		if heights.contains(&header.height) {
			locator.push(header.hash());
		}
		current = chain.get_block_header(&header.previous);
	}

	debug!(LOGGER, "sync: locator: {:?}", locator);

	Ok(locator)
}

// current height back to 0 decreasing in powers of 2
fn get_locator_heights(height: u64) -> Vec<u64> {
	let mut current = height.clone();
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

// Utility struct to group what information the main sync loop has to track
struct SyncInfo {
	prev_body_sync: (time::Tm, u64),
	prev_header_sync: (time::Tm, u64),
	prev_fast_sync: Option<time::Tm>,
	highest_height: u64,
}

impl SyncInfo {
	fn new() -> SyncInfo {
		let now = time::now_utc();
		SyncInfo {
			prev_body_sync: (now.clone(), 0),
			prev_header_sync: (now.clone(), 0),
			prev_fast_sync: None,
			highest_height: 0,
		}
	}

	fn header_sync_due(&mut self, header_head: &chain::Tip) -> bool {
		let now = time::now_utc();
		let (prev_ts, prev_height) = self.prev_header_sync;

		if header_head.height >= prev_height + (p2p::MAX_BLOCK_HEADERS as u64) - 4
			|| now - prev_ts > time::Duration::seconds(10)
		{
			self.prev_header_sync = (now, header_head.height);
			return true;
		}
		false
	}

	fn body_sync_due(&mut self, head: &chain::Tip) -> bool {
		let now = time::now_utc();
		let (prev_ts, prev_height) = self.prev_body_sync;

		if head.height >= prev_height + 96 || now - prev_ts > time::Duration::seconds(5) {
			self.prev_body_sync = (now, head.height);
			return true;
		}
		false
	}

	// For now this is a one-time thing (it can be slow) at initial startup.
	fn fast_sync_due(&mut self) -> bool {
		if let None = self.prev_fast_sync {
			let now = time::now_utc();
			self.prev_fast_sync = Some(now);
			true
		} else {
			false
		}
	}
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
			vec![
				10000, 9998, 9994, 9986, 9970, 9938, 9874, 9746, 9490, 8978, 7954, 5906, 1810, 0
			]
		);
	}
}
