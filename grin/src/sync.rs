// Copyright 2017 The Grin Developers
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

use std::thread;
use std::time::Duration;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use time;

use chain;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use p2p::{self, Peer, Peers, ChainAdapter};
use types::Error;
use util::LOGGER;

/// Starts the syncing loop, just spawns two threads that loop forever
pub fn run_sync(
	currently_syncing: Arc<AtomicBool>,
	peers: p2p::Peers,
	chain: Arc<chain::Chain>,
) {

	let chain = chain.clone();
	let _ = thread::Builder::new()
		.name("sync".to_string())
		.spawn(move || {
			let mut prev_body_sync = time::now_utc();
			let mut prev_header_sync = prev_body_sync.clone();

			// initial sleep to give us time to peer with some nodes
			thread::sleep(Duration::from_secs(30));

			loop {
				let syncing = needs_syncing(
					currently_syncing.clone(), peers.clone(), chain.clone());
				if syncing {

					let current_time = time::now_utc();

					// run the header sync every 10s
					if current_time - prev_header_sync > time::Duration::seconds(10) {
						header_sync(
							peers.clone(),
							chain.clone(),
						);
						prev_header_sync = current_time;
					}

					// run the body_sync every 5s
					if current_time - prev_body_sync > time::Duration::seconds(5) {
						body_sync(
							peers.clone(),
							chain.clone(),
						);
						prev_body_sync = current_time;
					}

					thread::sleep(Duration::from_secs(1));
				} else {
					thread::sleep(Duration::from_secs(10));
				}
			}
		});
}

fn body_sync(peers: Peers, chain: Arc<chain::Chain>) {

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

	// if we have 5 most_work_peers then ask for 50 blocks total (peer_count * 10)
	// max will be 80 if all 8 peers are advertising most_work
	let peer_count = {
		peers.most_work_peers().len()
	};
	let block_count = peer_count * 10;

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
			let peer = peers.most_work_peer();
			if let Some(peer) = peer {
				if let Ok(peer) = peer.try_read() {
					let _ = peer.send_block_request(hash);
				}
			}
		}
	}
}

pub fn header_sync(peers: Peers, chain: Arc<chain::Chain>) {
	if let Ok(header_head) = chain.get_header_head() {
		let difficulty = header_head.total_difficulty;

		if let Some(peer) = peers.most_work_peer() {
			if let Ok(p) = peer.try_read() {
				let peer_difficulty = p.info.total_difficulty.clone();
				if peer_difficulty > difficulty {
					let _ = request_headers(
						peer.clone(),
						chain.clone(),
					);
				}
			}
		}
	}
}

/// Request some block headers from a peer to advance us.
fn request_headers(
	peer: Arc<RwLock<Peer>>,
	chain: Arc<chain::Chain>,
) -> Result<(), Error> {
	let locator = get_locator(chain)?;
	if let Ok(peer) = peer.try_read() {
		debug!(
			LOGGER,
			"sync: request_headers: asking {} for headers, {:?}",
			peer.info.addr,
			locator,
		);
		let _ = peer.send_header_request(locator);
	} else {
		// not much we can do here, log and try again next time
		debug!(
			LOGGER,
			"sync: request_headers: failed to get read lock on peer",
		);
	}
	Ok(())
}


/// Whether we're currently syncing the chain or we're fully caught up and
/// just receiving blocks through gossip.
pub fn needs_syncing(
	currently_syncing: Arc<AtomicBool>,
	peers: Peers,
	chain: Arc<chain::Chain>) -> bool {

	let local_diff = peers.total_difficulty();
	let peer = peers.most_work_peer();

	// if we're already syncing, we're caught up if no peer has a higher
	// difficulty than us
	if currently_syncing.load(Ordering::Relaxed) {
		if let Some(peer) = peer {
			if let Ok(peer) = peer.try_read() {
				if peer.info.total_difficulty <= local_diff {
					info!(LOGGER, "synchronize stopped, at {:?} @ {:?}", local_diff, chain.head().unwrap().height);
					currently_syncing.store(false, Ordering::Relaxed);
				}
			}
		} else {
			info!(LOGGER, "sync: no peers available, disabling sync");
			currently_syncing.store(false, Ordering::Relaxed);
		}
	} else {
		if let Some(peer) = peer {
			if let Ok(peer) = peer.try_read() {
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
					currently_syncing.store(true, Ordering::Relaxed);
				}
			}
		}
	}
	currently_syncing.load(Ordering::Relaxed)
}

/// We build a locator based on sync_head.
/// Even if sync_head is significantly out of date we will "reset" it once we start getting
/// headers back from a peer.
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
		let next = 2u64.pow(heights.len() as u32);
		current = if current > next {
			current - next
		} else {
			0
		}
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
		// check the locator is still a manageable length, even for large numbers of headers
		assert_eq!(
			get_locator_heights(10000),
			vec![10000, 9998, 9994, 9986, 9970, 9938, 9874, 9746, 9490, 8978, 7954, 5906, 1810, 0]
		);
	}
}
