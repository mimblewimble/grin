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

use adapters::NetToChainAdapter;
use chain;
use core::core::hash::{Hash, Hashed};
use p2p::{self, Peer, NetAdapter};
use types::Error;
use util::LOGGER;

/// Starts the syncing loop, just spawns two threads that loop forever
pub fn run_sync(
	adapter: Arc<NetToChainAdapter>,
	p2p_server: Arc<p2p::Server>,
	chain: Arc<chain::Chain>,
) {
	let a_inner = adapter.clone();
	let p2p_inner = p2p_server.clone();
	let c_inner = chain.clone();
	let _ = thread::Builder::new()
		.name("body_sync".to_string())
		.spawn(move || {
			loop {
				if a_inner.is_syncing() {
					body_sync(p2p_inner.clone(), c_inner.clone());
				} else {
					thread::sleep(Duration::from_secs(5));
				}
			}
		});
	let _ = thread::Builder::new()
		.name("header_sync".to_string())
		.spawn(move || {
			loop {
				if adapter.is_syncing() {
					header_sync(adapter.clone(), p2p_server.clone(), chain.clone());
				} else {
					thread::sleep(Duration::from_secs(5));
				}
			}
		});
}

fn body_sync(
	p2p_server: Arc<p2p::Server>,
	chain: Arc<chain::Chain>,
) {
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

			// look back through the sync chain until we find a header
			// that is consistent with the height index (we know this is in the real chain)
			match chain.get_header_by_height(header.height) {
				Ok(height_header) => {
					if header.hash() == height_header.hash() {
						break;
					}
				},
				Err(_) => {},
			}

			hashes.push(header.hash());
			current = chain.get_block_header(&header.previous);
		}
	}
	hashes.reverse();

	let hashes_to_get = hashes
		.iter()
		.filter(|x| !chain.get_block(&x).is_ok())
		.take(10)
		.cloned()
		.collect::<Vec<_>>();

	if hashes_to_get.len() > 0 {
		debug!(
			LOGGER,
			"block_sync: requesting blocks ({}/{}), {:?}",
			body_head.height,
			header_head.height,
			hashes_to_get,
			);

		for hash in hashes_to_get.clone() {
			// TODO - what condition should we choose most_work_peer v random_peer (if any?)
			let peer = if hashes_to_get.len() < 100 {
				p2p_server.most_work_peer()
			} else {
				p2p_server.random_peer()
			};
			if let Some(peer) = peer {
				let peer = peer.read().unwrap();
				if let Err(e) = peer.send_block_request(hash) {
					debug!(LOGGER, "block_sync: error requesting block: {:?}, {:?}", hash, e);
				}
			}
		}
		thread::sleep(Duration::from_secs(1));
	} else {
		thread::sleep(Duration::from_secs(5));
	}
}

pub fn header_sync(
	adapter: Arc<NetToChainAdapter>,
	p2p_server: Arc<p2p::Server>,
	chain: Arc<chain::Chain>,
	) {
	debug!(LOGGER, "header_sync: loop");

	let difficulty = adapter.total_difficulty();

	if let Some(peer) = p2p_server.most_work_peer() {
		let peer = peer.clone();
		let p = peer.read().unwrap();
		let peer_difficulty = p.info.total_difficulty.clone();

		if peer_difficulty > difficulty {
			let _ = request_headers(
				peer.clone(),
				chain.clone(),
			);
		}
	}

	thread::sleep(Duration::from_secs(5));
}

/// Request some block headers from a peer to advance us.
fn request_headers(
	peer: Arc<RwLock<Peer>>,
	chain: Arc<chain::Chain>,
) -> Result<(), Error> {
	let locator = get_locator(chain)?;
	let peer = peer.read().unwrap();
	debug!(
		LOGGER,
		"sync: asking {} for headers, locator: {:?}",
		peer.info.addr,
		locator,
	);
	let _ = peer.send_header_request(locator);
	Ok(())
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
